use super::common;

use common::{assert_pt, local_options};
use ferri_pdf_dom::DocumentTree;
use ferri_pdf_style::{Style, Stylesheet, StylesheetSource};

#[test]
fn initial_disabled_state_reaches_support_queries_and_real_style_consumers() {
    for (extra, expected) in [
        ("", 13.0),
        (
            "body:has(#ordinary:disabled) p{font-size:19pt}body:has(#first:disabled) p{font-size:31pt}",
            19.0,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>{}fieldset{{display:none}}p{{margin:0;font-size:11pt}}@supports selector(:enabled) and selector(:disabled){{p{{font-size:13pt}}}}body:has(#first:enabled) p{{color:#123456}}{extra}</style><fieldset disabled><legend><input id=first></legend><input id=ordinary></fieldset><p>State consumer</p>",
            common::PROFILE
        );
        let output = crate::render_html(&html, &local_options()).unwrap();
        assert!(
            output.diagnostics.is_empty(),
            "valid initial-state selectors must not produce CSS diagnostics: {:?}",
            output.diagnostics
        );
        let trace = output.layout.unwrap();
        assert_eq!(trace.pages[0][0].text, "State consumer");
        assert_pt(trace.pages[0][0].font_size, expected);
        assert_eq!(trace.pages[0][0].runs[0].color.rgb, MARKED);
    }
}

const MARKED: [u8; 3] = [0x12, 0x34, 0x56];
const ELEMENTS: &str = "<!doctype html><p id=html>HTML</p><svg><g id=svg xlink:href='#x'></g><g id=plain href='#x'></g><foreignObject><p id=inside>Inside</p></foreignObject></svg><math><mi id=math>M</mi></math>";

const SCOPED_HTML: &str = "<!doctype html><body class=outside><section class=outer id=outer><p id=direct>Direct</p><section class=inner id=inner><p id=near>Near</p><div class=cut id=limit><p id=blocked>Blocked</p><section class=inner><p id=reentry>Reentry</p></section></div><p id=after>After</p></section></section><p id=outside>Outside</p>";

#[test]
fn custom_name_validation_is_shared_by_definedness_and_shadow_hosts() {
    assert_eq!(
        marked_sources(
            &[":defined{color:#123456}"],
            "<!doctype html><x!invalid-name id=invalid></x!invalid-name><x-valid id=valid></x-valid><font-face id=reserved></font-face>"
        ),
        ["invalid", "reserved"],
        "invalid PCEN characters and reserved custom names are ordinary uncustomized elements"
    );
}

#[test]
fn host_selector_arguments_respect_component_limits_on_the_normal_thread() {
    const COMPONENT_LIMIT: usize = 256;
    const ROOT_AND_HOST_ARGUMENT: usize = 2;
    for function in ["host", "host-context"] {
        for (logical_depth, allowed) in [
            (COMPONENT_LIMIT - ROOT_AND_HOST_ARGUMENT, true),
            (COMPONENT_LIMIT - ROOT_AND_HOST_ARGUMENT + 1, false),
        ] {
            let selector = format!(
                ":{function}({}.themed{})",
                ":is(".repeat(logical_depth),
                ")".repeat(logical_depth)
            );
            let html = format!(
                "<!doctype html><style>{}</style><div class=themed><template shadowrootmode=open><style>{selector}{{font-size:19pt}}</style><p>Boundary</p></template></div>",
                common::PROFILE
            );
            let result = crate::render_html(&html, &local_options());
            if allowed {
                let trace = result
                    .expect("a host argument at the inclusive component limit parses and matches")
                    .layout
                    .unwrap();
                assert_pt(trace.pages[0][0].font_size, 19.0);
            } else {
                assert!(
                    matches!(
                        result,
                        Err(crate::Error::Css(
                            ferri_pdf_style::StyleError::GrammarLimit {
                                limit: COMPONENT_LIMIT,
                                ..
                            }
                        ))
                    ),
                    "one more nested argument must return the existing CSS limit error"
                );
            }
        }
    }
}

#[test]
fn shadow_hosts_are_featureless_and_arguments_use_the_normal_tree() {
    for (css, expected) in [
        (
            ":host{font-size:19pt}.themed.themed{font-size:31pt}:scope{font-size:41pt}",
            19.0,
        ),
        (
            ":host(.themed){font-size:23pt}:host{font-size:19pt}:host-context(body){font-size:29pt}",
            23.0,
        ),
        (
            ":host{font-size:19pt}:host(:host){font-size:31pt}:host-context(:host){font-size:37pt}",
            19.0,
        ),
        (
            ":host{font-size:19pt}:host-context(body){font-size:29pt}",
            29.0,
        ),
        (":host(:is(body .themed)){font-size:23pt}", 23.0),
        (
            ":host{font-size:19pt}@supports selector(:host-context(body div)){:host{font-size:31pt}}",
            19.0,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>{}</style><div class=themed><template shadowrootmode=open><style>{css}</style><p>Shadow host</p></template></div><p>Outside</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let lines: Vec<_> = trace.pages.iter().flatten().collect();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Shadow host", "Outside"]
        );
        assert_pt(lines[0].font_size, expected);
        assert_pt(lines[1].font_size, 12.0);
    }
}

#[test]
fn shadow_scopes_anchor_at_the_host_without_exposing_light_ancestors() {
    for (css, host_size, child_size) in [
        ("@scope(:host){p{font-size:19pt}}", 12.0, 19.0),
        ("@scope(:host){:scope{font-size:21pt}}", 21.0, 21.0),
        (
            "@scope{:scope{font-size:23pt}:scope>p{font-size:29pt}}",
            23.0,
            29.0,
        ),
        ("@scope(:host){:scope{&{font-size:23pt}}}", 23.0, 23.0),
    ] {
        let html = format!(
            "<!doctype html><style>{}#host::before{{content:'Host';display:block}}</style><section><div id=host><template shadowrootmode=open><style>{css}</style><p>Shadow child</p></template></div></section><p>Outside</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let lines: Vec<_> = trace.pages.iter().flatten().collect();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Host", "Shadow child", "Outside"]
        );
        assert_pt(lines[0].font_size, host_size);
        assert_pt(lines[1].font_size, child_size);
        assert_pt(lines[2].font_size, 12.0);
    }
}

#[test]
fn host_context_walks_shadow_including_ancestors_in_their_own_context() {
    let html = format!(
        "<!doctype html><style>{}</style><section class=theme><div><template shadowrootmode=open><div><template shadowrootmode=closed><style>:host-context(.theme){{font-size:29pt}}:host-context(:host){{font-size:37pt}}</style><p>Nested</p></template></div></template></div></section>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let line = trace
        .pages
        .iter()
        .flatten()
        .find(|line| line.text == "Nested")
        .unwrap();
    assert_pt(line.font_size, 29.0);
}

#[test]
fn host_context_nesting_retains_parent_list_specificity() {
    let html = format!(
        "<!doctype html><style>{}</style><section class=theme><div><template shadowrootmode=open><style>#missing,.theme{{:host-context(&){{font-size:23pt}}}}:host-context(.theme){{font-size:19pt}}</style><p>Nested argument</p></template></div></section>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let line = trace.pages.iter().flatten().next().unwrap();
    assert_eq!(line.text, "Nested argument");
    assert_pt(line.font_size, 23.0);
}

#[test]
fn shadow_host_slotted_and_part_selectors_respect_tree_contexts() {
    let html = format!(
        "<!doctype html><style>{}x-panel{{display:block}}x-panel::part(label){{font-size:23pt}}p{{color:red}}</style><x-panel class=themed><template shadowrootmode=open><style>:host(.themed){{color:#123456}}:host-context(body) .label{{font-size:19pt}}::slotted(.picked){{font-size:29pt}}slot:has-slotted{{color:green}}</style><p class=label part=label>Part</p><slot name=n></slot></template><p class=picked slot=n>Slotted</p></x-panel><p>Outside</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Part", "Slotted", "Outside"]
    );
    assert_pt(lines[0].font_size, 23.0);
    assert_eq!(
        lines[0].color.rgb, MARKED,
        "document p selectors cannot cross a shadow boundary"
    );
    assert_pt(lines[1].font_size, 29.0);
    assert_eq!(
        lines[1].color.rgb,
        [255, 0, 0],
        "slotted elements retain their light-tree author rules"
    );
    assert_eq!(lines[2].color.rgb, [255, 0, 0]);
}

#[test]
fn slot_aliases_use_the_source_tree_and_keep_ordinary_selectors_inside_it() {
    let html = format!(
        "<!doctype html><style>{}</style><div><template shadowrootmode=open><style>::slotted(.picked){{font-size:29pt;color:#123456}}.picked,::slotted(.absent){{font-size:41pt}}slot::slotted(:host){{font-size:47pt}}</style><slot></slot></template><p class=picked>Slotted</p></div><p class=picked>Outside</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Slotted", "Outside"]
    );
    assert_pt(lines[0].font_size, 29.0);
    assert_eq!(lines[0].color.rgb, MARKED);
    assert_pt(lines[1].font_size, 12.0);
}

#[test]
fn compound_alias_arguments_allow_complex_selectors_in_nested_functions() {
    for argument in [
        ":is(body .picked)",
        ":not(.other .picked)",
        ":has(.a .b)",
        ":nth-child(1 of body .picked)",
    ] {
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode=open><style>::slotted({argument}){{font-size:29pt}}</style><slot></slot></template><p class=picked><span class=a><span class=b>Nested argument</span></span></p></div><p class=picked>Outside</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let lines: Vec<_> = trace.pages.iter().flatten().collect();
        assert_eq!(
            lines[0].runs[0].font_size, 29.0,
            "valid nested complex selector in {argument}"
        );
        assert_pt(lines[1].font_size, 12.0);
    }
}

#[test]
fn part_aliases_keep_the_originating_host_featureless_in_its_own_root() {
    let html = format!(
        "<!doctype html><style>{}</style><div class=themed><template shadowrootmode=open><style>:host::part(label){{font-size:23pt}}.themed::part(label){{font-size:31pt}}</style><p part=label>Part</p></template></div>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_pt(trace.pages[0][0].font_size, 23.0);
}

#[test]
fn scoped_aliases_use_the_originating_host_or_slot_for_roots_and_proximity() {
    let html = format!(
        "<!doctype html><style>{}@scope(.near){{x-host::part(label){{font-size:29pt}}}}@scope(.outer){{x-host::part(label){{font-size:23pt}}}}</style><section class=outer><div class=near><x-host><template shadowrootmode=open><style>@scope(.inner){{slot::slotted(.picked){{font-size:31pt}}.picked{{font-size:41pt}}}}</style><div class=inner><p part=label>Part</p><slot></slot></div></template><p class=picked>Slotted</p></x-host></div></section><p>Outside</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Part", "Slotted", "Outside"]
    );
    for (line, expected) in lines.iter().zip([29.0, 31.0, 12.0]) {
        assert_pt(line.font_size, expected);
    }
}

#[test]
fn alias_rules_generate_tree_abiding_pseudo_elements_on_the_selected_node() {
    let html = format!(
        "<!doctype html><style>{}x-host::part(label)::before{{content:'Part before';display:block;font-size:23pt}}</style><x-host><template shadowrootmode=open><style>::slotted(.picked)::after{{content:'Slotted after';display:block;font-size:29pt}}</style><p part=label>Part</p><slot></slot></template><p class=picked>Slotted</p></x-host>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Part before", "Part", "Slotted", "Slotted after"]
    );
    assert_pt(lines[0].runs[0].font_size, 23.0);
    assert_pt(lines[3].runs[0].font_size, 29.0);
}

#[test]
fn slot_presence_uses_flattened_nodes_including_text_and_hidden_elements() {
    for (light, fallback, expected) in [
        ("", "", false),
        ("<!--comment-->", "", false),
        (" ", "", true),
        ("<span hidden>Hidden</span>", "", true),
        ("", "<span>Fallback</span>", true),
    ] {
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode=open><style>slot:has-slotted+p{{font-size:19pt}}</style><slot id=probe>{fallback}</slot><p>Presence</p></template>{light}</div>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Presence")
            .unwrap();
        assert_pt(line.font_size, if expected { 19.0 } else { 12.0 });
    }
    for (fallback, expected) in [("", false), ("<span>Fallback</span>", true)] {
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode=open><div><template shadowrootmode=open><style>slot:has-slotted+p{{font-size:19pt}}</style><slot></slot><p>Presence</p></template><slot>{fallback}</slot></div></template></div>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Presence")
            .unwrap();
        assert_pt(line.font_size, if expected { 19.0 } else { 12.0 });
    }
}

#[test]
fn slotted_aliases_flatten_fallback_and_reprojected_slots() {
    for html in [
        "<div><template shadowrootmode=open><style>::slotted(.picked){font-size:29pt}</style><slot><p class=picked>Fallback</p></slot></template></div>",
        "<div><template shadowrootmode=open><div><template shadowrootmode=open><style>::slotted(.picked){font-size:29pt}</style><slot></slot></template><slot><p class=picked>Fallback</p></slot></div></template></div>",
        "<div><template shadowrootmode=open><style>::slotted(slot){font-size:29pt}</style><slot></slot></template><slot>Fallback</slot></div>",
    ] {
        let html = format!("<!doctype html><style>{}</style>{html}", common::PROFILE);
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Fallback")
            .unwrap();
        assert_eq!(
            line.runs[0].font_size, 29.0,
            "flattened slot aliases in {html}"
        );
    }
}

#[test]
fn nested_fallback_slots_match_each_origin_with_its_own_scope() {
    for (css, expected) in [
        ("#outer::slotted(p){font-size:29pt}", 29.0),
        (
            "#outer::slotted(p){font-size:29pt}@scope(#inner){#outer::slotted(p){font-size:41pt}}",
            29.0,
        ),
        (
            "@scope(#inner){:scope#inner::slotted(p){font-size:31pt}}@scope(#outer){:scope#outer::slotted(p){font-size:29pt}}",
            29.0,
        ),
        (
            "@scope(#outer){:scope#outer::slotted(p){font-size:29pt}}@scope(#inner){:scope#inner::slotted(p){font-size:31pt}}",
            31.0,
        ),
        (
            "@scope(#outer) to (#inner){:scope#outer::slotted(p){font-size:29pt}}",
            29.0,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode=open><style>{css}</style><slot id=outer><slot id=inner><p>Nested fallback</p></slot></slot></template></div>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Nested fallback")
            .unwrap();
        assert_eq!(
            line.runs[0].font_size, expected,
            "alias origin and scope for {css}"
        );
    }
}

#[test]
fn host_part_aliases_include_forwarded_parts_of_their_own_root() {
    for css in [
        ":host::part(exposed){font-size:29pt}",
        "@scope(:host){:scope::part(exposed){font-size:29pt}}",
    ] {
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode=open><style>{css}</style><x-inner exportparts='label:exposed'><template shadowrootmode=open><p part=label>Forwarded</p></template></x-inner></template></div>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Forwarded")
            .unwrap();
        assert_pt(line.runs[0].font_size, 29.0);
    }
}

#[test]
fn slotted_arguments_respect_component_limits_on_the_normal_thread() {
    const COMPONENT_LIMIT: usize = 256;
    const ROOT_AND_ARGUMENT: usize = 2;
    for (depth, allowed) in [
        (COMPONENT_LIMIT - ROOT_AND_ARGUMENT, true),
        (COMPONENT_LIMIT - ROOT_AND_ARGUMENT + 1, false),
    ] {
        let selector = format!(
            "::slotted({}.picked{})",
            ":is(".repeat(depth),
            ")".repeat(depth)
        );
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode=open><style>{selector}{{font-size:29pt}}</style><slot></slot></template><p class=picked>Boundary</p></div>",
            common::PROFILE
        );
        let result = crate::render_html(&html, &local_options());
        if allowed {
            let trace = result
                .expect("the existing inclusive slotted component limit parses and matches")
                .layout
                .unwrap();
            assert_pt(trace.pages[0][0].font_size, 29.0);
        } else {
            assert!(
                matches!(
                    result,
                    Err(crate::Error::Css(
                        ferri_pdf_style::StyleError::GrammarLimit {
                            limit: COMPONENT_LIMIT,
                            ..
                        }
                    ))
                ),
                "another slotted argument level must return the typed CSS limit error"
            );
        }
    }
}

#[test]
fn exported_parts_keep_many_aliases_across_nested_roots() {
    let html = format!(
        "<!doctype html><style>{}x-outer::part(exposed){{font-size:23pt}}x-outer::part(exposed active){{color:#123456}}.leak,x-outer::part(missing){{font-size:41pt}}</style><x-outer><template shadowrootmode=open><x-middle exportparts='intermediate:exposed, state:active'><template shadowrootmode=closed><x-inner exportparts='alpha:intermediate, beta:intermediate, selected:state'><template shadowrootmode=open><p class=leak part='alpha selected'>First</p><p class=leak part=beta>Second</p><p class=leak part=private>Private</p></template></x-inner></template></x-middle></template></x-outer><p>Outside</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second", "Private", "Outside"]
    );
    for (line, expected) in lines.iter().zip([23.0, 23.0, 12.0, 12.0]) {
        assert_pt(line.font_size, expected);
    }
    assert_eq!(lines[0].color.rgb, MARKED);
    assert_ne!(
        lines[1].color.rgb, MARKED,
        "part-name intersections apply to the same element"
    );
}

#[test]
fn scope_roots_limits_and_proximity_choose_winner() {
    for (css, expected) in [
        (
            "@scope (.outer) to (.cut){p{color:#123456}}",
            vec!["direct", "near", "after"],
        ),
        (
            "@scope (.outer,.inner) to (.cut){>p{color:#123456}}",
            vec!["direct", "near", "reentry", "after"],
        ),
        (
            "@scope (.inner){p{color:#123456}}@scope (.outer){p{color:red}}p{color:blue}",
            vec!["near", "blocked", "reentry", "after"],
        ),
        (
            "@scope (.outer){p{color:red}}@scope (.outer){p{color:#123456}}",
            vec!["direct", "near", "blocked", "reentry", "after"],
        ),
        ("@scope (.outer) to (:scope){p{color:#123456}}", vec![]),
        ("@scope (.outer){:scope{color:#123456}}", vec!["outer"]),
        ("@scope (.outer){section{color:#123456}}", vec!["inner"]),
        (
            "@scope (.outer) to (.outside :scope > .inner){p{color:#123456}}",
            vec!["direct"],
        ),
        (
            "@scope (.outer){.outside :scope > p{color:#123456}}",
            vec!["direct"],
        ),
    ] {
        assert_eq!(
            marked_sources(&[css], SCOPED_HTML),
            expected,
            "scope roots, bounds and proximity: {css}"
        );
    }
}

#[test]
fn ownerless_scope_uses_a_featureless_document_root() {
    const HTML: &str =
        "<!doctype html><html id=html><body id=body><p id=child>Child</p></body></html>";
    for (css, expected) in [
        ("@scope{html{color:#123456}}", vec!["html"]),
        (
            "@scope{>html{color:#123456}>body{color:#123456}}",
            vec!["html"],
        ),
        ("@scope{:scope{color:#123456}}", vec![]),
        ("@scope{&{color:#123456}}", vec![]),
        ("@scope{:scope>html{color:#123456}}", vec!["html"]),
        ("@scope{:is(:scope)>html{color:#123456}}", vec!["html"]),
        ("@scope{*>html{color:#123456}}", vec![]),
        ("@scope(html){p{color:red}}@scope{p{color:#123456}}", vec![]),
        ("@scope to (:scope){html{color:#123456}}", vec!["html"]),
    ] {
        assert_eq!(
            marked_sources(&[css], HTML),
            expected,
            "ownerless document scope must preserve virtual-root matching and proximity: {css}"
        );
    }
}

#[test]
fn scope_in_custom_selector_arguments_suppresses_implicit_descendants() {
    assert_eq!(
        marked_sources(
            &["@scope (p){:is(:current(:scope),p){color:#123456}}"],
            "<!doctype html><p id=root>Root</p>"
        ),
        ["root"],
        "a lexical :scope inside a supported selector argument makes this nonrelative even when :current is statically inactive"
    );
}

#[test]
fn nested_scope_and_nesting_selector_context() {
    for (css, expected) in [
        (
            "@scope (.outer) to (.cut){@scope (.inner){p{color:#123456}}}",
            vec!["near", "after"],
        ),
        (
            "@scope (.outer){@scope (:scope > .inner) to (:scope > .cut){>p{color:#123456}}}",
            vec!["near", "after"],
        ),
        (
            ".outer{@scope (& > .inner) to (& .cut){& > p{color:#123456}}}",
            vec!["near", "after"],
        ),
        (
            "@scope (.outer){.inner{& > p{color:#123456}}}",
            vec!["near", "reentry", "after"],
        ),
        (
            "@scope (.outer){@media print{@layer example{>p{color:#123456}}}}",
            vec!["direct"],
        ),
        (
            "@namespace h 'http://www.w3.org/1999/xhtml';@scope (h|section.outer){>h|p{color:#123456}}",
            vec!["direct"],
        ),
    ] {
        assert_eq!(
            marked_sources(&[css], SCOPED_HTML),
            expected,
            "nested scope and selector context: {css}"
        );
    }
}

#[test]
fn scope_does_not_inflate_specificity() {
    for (css, expected) in [
        (
            "@scope (#outer){p{color:#123456}}.outer p{color:red}",
            vec![],
        ),
        (
            "@scope (#outer){& > p{color:#123456}}section > p{color:red}",
            vec![],
        ),
        (
            "@scope (#outer){:scope > p{color:#123456}}section > p{color:red}",
            vec!["direct"],
        ),
        (
            "@scope (.outer){p{color:#123456}}p{color:red}",
            vec!["direct", "near", "blocked", "reentry", "after"],
        ),
        (
            "@scope (.inner){p{color:#123456}}@scope (.outer){p#near{color:red}}",
            vec!["blocked", "reentry", "after"],
        ),
        (
            "@layer early,late;@layer early{@scope (.inner){p{color:#123456}}}@layer late{p{color:red}}",
            vec![],
        ),
    ] {
        assert_eq!(
            marked_sources(&[css], SCOPED_HTML),
            expected,
            "specificity/layers precede scope proximity, and scoped ampersands have zero specificity: {css}"
        );
    }
}

#[test]
fn implicit_scope_uses_stylesheet_owner_and_keeps_generated_targets() {
    let html = format!(
        "<!doctype html><style>{}p{{margin:0;font-size:12pt;color:black}}</style><div><style>@scope{{>p{{font-size:19pt;color:#123456}}&>p::before{{content:'Scoped ';}}}}</style><p>Inside</p></div><p>Outside</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let blocks: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["Scoped Inside", "Outside"],
        "implicit scope must use the containing style element's parent, including generated content"
    );
    assert_pt(blocks[0].font_size, 19.0);
    assert!(blocks[0].runs.iter().all(|run| run.color.rgb == MARKED));
    assert_pt(blocks[1].font_size, 12.0);
    assert_eq!(blocks[1].runs[0].color.rgb, [0, 0, 0]);
}

#[test]
fn imported_and_linked_implicit_scopes_follow_the_source_owner() {
    const SCOPED_SHEET: &str = "data:text/css,@scope%7Bp%7Bfont-size:19pt%7D%7D";
    for (source, outside_size) in [
        (format!("<style>@import '{SCOPED_SHEET}';</style>"), 12.0),
        (format!("<link rel=stylesheet href='{SCOPED_SHEET}'>"), 12.0),
    ] {
        let html = format!(
            "<!doctype html><style>{}p{{margin:0;font-size:12pt}}</style><div>{source}<p>Inside</p></div><p>Outside</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let blocks: Vec<_> = trace.pages.iter().flatten().collect();
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>(),
            ["Inside", "Outside"]
        );
        assert_pt(blocks[0].font_size, 19.0);
        assert_pt(blocks[1].font_size, outside_size);
    }
}

#[test]
fn scope_keeps_named_fonts_and_layer_registration_global() {
    const FAMILY: &str = "ScopeGlobalFace";
    let html = format!(
        "<!doctype html><style>{}@scope (#absent){{@font-face{{font-family:{FAMILY};src:url('../assets/DejaVuSans.ttf')}}@layer early;}}@layer late,early;@layer early{{p{{font-size:19pt}}}}@layer late{{p{{font-size:23pt}}}}p{{font-family:{FAMILY}}}</style><p>Global definitions</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_pt(trace.pages[0][0].font_size, 23.0);
    assert!(
        trace.fonts.iter().any(|font| font.family == FAMILY),
        "a global face declared inside an unmatched scope must be usable outside it"
    );
}

#[test]
fn invalid_scope_boundaries_recover_and_support_probes_are_honest() {
    for css in [
        "@scope (.outer::before){p{color:red}}p{color:#123456}",
        "@scope (.outer) to (p::after){p{color:red}}p{color:#123456}",
        "@scope (.outer,:unknown){p{color:red}}p{color:#123456}",
        "@scope (.outer) trailing{p{color:red}}p{color:#123456}",
    ] {
        assert_eq!(
            marked_sources(
                &[css],
                "<!doctype html><section class=outer><p id=sample>Sample</p></section>"
            ),
            ["sample"],
            "invalid scope must not broaden or consume its valid sibling: {css}"
        );
    }
    for (probe, supported) in [
        ("@scope (.outer) to (.cut){>p{color:red}}", true),
        (".outer{@scope (&>.inner){&>p{color:red}}}", true),
        ("@scope (.outer::before){p{color:red}}", false),
        ("@scope (.outer) to (::after){p{color:red}}", false),
        ("@scope (.outer){p{display:ruby}}", false),
        ("@scope (.outer){@import 'unfetched.css';}", false),
        ("@scope (.outer){color:red}", false),
    ] {
        let html = format!(
            "<!doctype html><style>{}p{{font-size:12pt}}@supports-condition --scoped{{{probe}}}@supports (--scoped){{p{{font-size:19pt}}}}</style><p>Probe</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_pt(
            trace.pages[0][0].font_size,
            if supported { 19.0 } else { 12.0 },
        );
    }
}

#[test]
fn nesting_ampersand_lists_and_specificity() {
    const HTML: &str = "<!doctype html><div class=parent id=outer><p id=direct>Direct</p><section><p id=deep>Deep</p></section><div class=parent id=inner><p id=inside>Inside</p></div></div><p id=sibling>Sibling</p>";
    for (css, expected) in [
        (
            ".parent{p{color:#123456}}",
            vec!["direct", "deep", "inside"],
        ),
        (".parent{>p{color:#123456}}", vec!["direct", "inside"]),
        (".parent{& + p{color:#123456}}", vec!["sibling"]),
        (".parent{body &{color:#123456}}", vec!["outer", "inner"]),
        (".parent{& &{color:#123456}}", vec!["inner"]),
        (
            "#missing,.parent{& p{color:#123456}}.parent p{color:red}",
            vec!["direct", "deep", "inside"],
        ),
        (
            ".parent::before,.parent{& p{color:#123456}}.parent p{color:red}",
            vec!["direct", "deep", "inside"],
        ),
        (
            "#missing,.parent{:where(&) p{color:red}}.parent p{color:#123456}",
            vec!["direct", "deep", "inside"],
        ),
        (".parent{&:has(>p){color:#123456}}", vec!["outer", "inner"]),
        (
            ".parent{:is(:unknown(&),.parent){color:#123456}}",
            vec!["outer", "inner"],
        ),
        (
            ".parent{:is(:current(&),p){color:#123456}}",
            vec!["direct", "deep", "inside", "sibling"],
        ),
    ] {
        assert_eq!(
            marked_sources(&[css], HTML),
            expected,
            "nested selector matching/specificity: {css}"
        );
    }
    assert_eq!(
        marked_sources(
            &["html{color:#123456}&{color:red}"],
            "<!doctype html><html id=root><body><p id=child>Child</p></body></html>"
        ),
        ["root"],
        "top-level ampersand means scope with zero specificity"
    );
}

#[test]
fn nested_declarations_preserve_interleaved_source_order() {
    for css in [
        "p{color:red;&{color:blue}color:#123456}",
        "p{color:red;@media print{color:blue}color:#123456}",
        "p{color:red;@media print{&{color:blue}color:#123456}}",
    ] {
        assert_eq!(
            marked_sources(&[css], "<!doctype html><p id=sample>Sample</p>"),
            ["sample"],
            "declaration runs remain in source order: {css}"
        );
    }
    assert_eq!(
        marked_sources(
            &["#missing,p{color:red;@media print{color:#123456}}.parent p{color:red}"],
            "<!doctype html><div class=parent><p id=sample>Sample</p></div>"
        ),
        Vec::<String>::new(),
        "nested declarations keep the matching parent's specificity rather than the greatest list specificity"
    );
    let html = format!(
        "<!doctype html><style>{}p{{&::before{{content:'Before '}}}}p,p::before{{color:red;@media print{{color:#123456}}}}p::after{{content:' After';color:red;&{{color:#123456}}}}</style><p>Body</p>",
        common::PROFILE
    );
    let trace = crate::render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let runs = &trace.pages[0][0].runs;
    assert_eq!(trace.pages[0][0].text, "Before Body After");
    assert_eq!(
        runs.iter().map(|run| run.color.rgb).collect::<Vec<_>>(),
        [MARKED, MARKED, [255, 0, 0]],
        "nested declarations include pseudo-elements, while an explicit ampersand cannot represent one"
    );
}

#[test]
fn nested_conditionals_and_invalid_selectors_recover() {
    const HTML: &str = "<!doctype html><section class=parent><p id=child>Child</p></section><p id=outside>Outside</p>";
    for css in [
        ".parent{@media print{@supports (color:red){>p{color:#123456}}}}",
        ".parent{@media screen{p{color:red}}@media print{p{color:#123456}}}",
        "@layer early,late;@layer early{.parent{p{color:red}}}@layer late{.parent{p{color:#123456}}}",
        ".parent{&bad{color:red}p{color:#123456}}",
        ".parent{p,:unknown{color:red}p{color:#123456}}",
        ".parent{--tokens:{not:a-rule};p{color:#123456}}",
    ] {
        assert_eq!(
            marked_sources(&[css], HTML),
            ["child"],
            "nested conditions/recovery: {css}"
        );
    }
    assert_eq!(
        marked_sources(
            &["@namespace h 'http://www.w3.org/1999/xhtml';h|section{h|p{color:#123456}}"],
            HTML
        ),
        ["child"],
        "nested selectors inherit the declaring namespace environment"
    );
    assert_eq!(
        marked_sources(
            &[".parent{@layer late,early;@layer early{p{color:red}}@layer late{p{color:#123456}}}"],
            HTML
        ),
        Vec::<String>::new(),
        "nested layer statements establish ordering before the subsequent nested blocks"
    );
}

#[test]
fn named_support_blocks_validate_the_actual_nested_rule_context() {
    for (probe, supported) in [
        ("article{>p{color:red}}", true),
        (
            "article{color:red;@media print{color:blue;>p{color:red}}}",
            true,
        ),
        (
            "article{@font-face{font-family:X;src:url(unused.ttf)}}",
            false,
        ),
        ("article{@layer child;color:red}", true),
        ("article{&bad{color:red}}", false),
        ("article{p{display:ruby}}", false),
    ] {
        let html = format!(
            "<!doctype html><style>{}@supports-condition --nested{{{probe}}}p{{font-size:12pt}}@supports (--nested){{p{{font-size:19pt}}}}</style><p>Probe</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages[0][0].font_size,
            if supported { 19.0 } else { 12.0 },
            "nested support probe: {probe}"
        );
    }
}

fn marked_sources(sources: &[&str], html: &str) -> Vec<String> {
    let sources: Vec<_> = std::iter::once(&"[id]{display:block}")
        .chain(sources.iter())
        .map(|source| StylesheetSource {
            text: (*source).into(),
            base: Default::default(),
        })
        .collect();
    let sheet = Stylesheet::parse(&sources).expect("recoverable selector stylesheet");
    let tree = DocumentTree::parse(html).expect("parse test HTML");
    tree.elements()
        .filter_map(|element| {
            let id = element.attr("id")?;
            let style = sheet
                .computed(element, &Style::default())
                .expect("compute a selected element");
            (style.color.rgb == MARKED).then(|| id.to_owned())
        })
        .collect()
}

#[test]
fn namespace_queries_share_the_complete_sheet_environment() {
    for css in [
        "@namespace h 'http://www.w3.org/1999/xhtml';@supports selector(h|p){p{color:#123456}}",
        "@supports-condition --html{h|p{color:red}}@namespace h 'http://www.w3.org/1999/xhtml';@supports (--html){p{color:#123456}}",
        "@namespace h 'http://invalid.test';@supports-condition --html{h|p{color:red}}@namespace h 'http://www.w3.org/1999/xhtml';@supports (--html){p{color:#123456}}",
    ] {
        assert_eq!(
            marked_sources(&[css], ELEMENTS),
            ["html", "inside"],
            "sheet-local query context: {css}"
        );
    }
}

#[test]
fn namespace_placement_and_imports_preserve_sheet_boundaries() {
    for (css, expected) in [
        (
            "@unknown;@namespace h 'http://www.w3.org/1999/xhtml';h|p{color:#123456}",
            vec!["html", "inside"],
        ),
        (
            "p{color:red}@namespace h 'http://www.w3.org/1999/xhtml';h|p{color:#123456}",
            vec![],
        ),
        (
            "@media print{@namespace h 'http://www.w3.org/1999/xhtml';h|p{color:#123456}}",
            vec![],
        ),
        (
            "@namespace h 'http://www.w3.org/1999/xhtml';@media print{h|p{color:#123456}}",
            vec!["html", "inside"],
        ),
        ("@namespace h 'bad' extra;h|p{color:#123456}", vec![]),
        (
            "@namespace h 'http://www.w3.org/1999/xhtml';@namespace h url(bad extra);h|p{color:#123456}",
            vec!["html", "inside"],
        ),
    ] {
        assert_eq!(
            marked_sources(&[css], ELEMENTS),
            expected,
            "namespace placement/recovery: {css}"
        );
    }
    let html = "<!doctype html><style>@import 'data:text/css,p%7Bfont-size:19pt%7Dh%7Cp%7Bfont-size:23pt%7D';@namespace h 'http://www.w3.org/1999/xhtml';</style><p>Separate</p>";
    let trace = crate::render_html(html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_pt(trace.pages[0][0].font_size, 19.0);
}

#[test]
fn language_selectors_canonicalize_tags_and_filter_extended_ranges() {
    for (tag, range, expected) in [
        ("iw", "he", true),
        ("en-BU", "en-MM", true),
        ("cmn-CN", "zh", true),
        ("zh-cmn-Hans", "cmn", true),
        ("ar-ajp", "ar-apc", true),
        ("sgn-BE-FR", "sgn", true),
        ("de-Latn-DE", "de-DE", true),
        ("de-Latn-CH", "*-CH", true),
        ("en-b-bbb-a-aaa", "en-a-aaa-b-bbb", true),
        ("en-x-private", "en-private", false),
        ("xyzzy", "xyzzy", true),
        ("sh", "sr-Latn", false),
        ("en", "en_US", false),
        ("", "*", false),
        ("und", "*", true),
        ("", "", true),
    ] {
        let html = format!("<!doctype html><p id=sample lang='{tag}'>Sample</p>");
        assert_eq!(
            marked(&format!(":lang('{range}')"), &html),
            if expected { vec!["sample"] } else { vec![] },
            "RFC filtering: {tag:?} against {range:?}"
        );
    }
    assert_eq!(
        marked(
            ":lang(en)",
            "<!doctype html><div lang=en><p id=html xml:lang=fr>HTML</p><svg lang=en xml:lang=fr><text id=svg>SVG</text></svg></div>"
        ),
        ["html"],
        "literal HTML xml:lang has no effect; the XML namespace wins in SVG"
    );
    assert_eq!(
        marked(
            ":lang(fr)",
            "<!doctype html><meta http-equiv=content-language content=en><meta http-equiv=content-language content='fr extra'><meta http-equiv=content-language content='de,it'><p id=inherited>Default</p><p id=empty lang=''>Unknown</p>"
        ),
        ["inherited"],
        "last successful pragma supplies the default, while explicit empty lang stops inheritance"
    );
}

#[test]
fn direction_uses_host_semantics_and_excludes_isolated_descendants() {
    for (html, rtl) in [
        ("<p dir=auto id=sample><bdi>אב</bdi>Latin</p>", false),
        (
            "<p dir=auto id=sample><span dir=rtl>אב</span>Latin</p>",
            false,
        ),
        ("<p dir=auto id=sample><script>Latin</script>אב</p>", true),
        ("<div dir=rtl><bdi id=sample>123</bdi></div>", false),
        (
            "<div dir=rtl><input type=tel id=sample value=אב></div>",
            false,
        ),
        ("<input type=unknown dir=auto id=sample value=אב>", true),
        ("<textarea dir=auto id=sample>אב</textarea>", true),
    ] {
        assert_eq!(
            marked(
                if rtl { ":dir(rtl)" } else { ":dir(ltr)" },
                &format!("<!doctype html>{html}")
            ),
            ["sample"],
            "initial direction: {html}"
        );
    }
    assert_eq!(
        marked(":dir(sideways)", "<p id=sample>Text</p>"),
        [] as [String; 0]
    );
}

#[test]
fn blank_controls_use_sanitization_and_initial_selection() {
    for (attributes, blank) in [
        ("type=text value='&#10;&#13;'", true),
        ("type=text value=' '", false),
        ("type=url value=' &#10; '", true),
        ("type=email value=' &#13; '", true),
        ("type=number value=+1", true),
        ("type=number value=1.", true),
        ("type=number value=.5", false),
        ("type=number value=1e999", true),
        ("type=date value=1900-02-29", true),
        ("type=date value=2000-02-29", false),
        ("type=month value=0000-12", true),
        ("type=month value=2026-12", false),
        ("type=week value=2020-W53", false),
        ("type=week value=2021-W53", true),
        ("type=time value=24:00", true),
        ("type=time value=23:59:59.999", false),
        ("type=datetime-local value='2026-01-31 12:00'", false),
        ("type=file value=forged", true),
        ("type=hidden", false),
        ("type=checkbox", true),
        ("type=checkbox checked", false),
    ] {
        assert_eq!(
            marked(
                ":blank",
                &format!("<!doctype html><input id=sample {attributes}>")
            ),
            if blank { vec!["sample"] } else { vec![] },
            "initial blankness: {attributes}"
        );
    }
    assert_eq!(
        marked(
            ":blank",
            "<!doctype html><form><input id=first type=radio name=x checked><input id=last type=radio name=x checked></form>"
        ),
        ["first"],
        "the last initially checked radio wins its form/name group"
    );
    assert_eq!(
        marked(
            ":blank",
            "<!doctype html><select id=empty><option value=''>Choose</option><option selected value=''>Empty</option></select><select id=filled><option disabled value=''>Disabled</option><option> Filled </option></select><select id=multiple multiple><option>A</option></select>"
        ),
        ["empty", "multiple"],
        "selection and option values determine blankness"
    );
}

#[test]
fn target_resolution_checks_literal_fragments_before_decoding() {
    for (fragment, html, expected) in [
        (
            "%74arget",
            "<p id=target>Decoded</p><p id='%74arget'>Literal</p>",
            vec!["%74arget"],
        ),
        (
            "%74arget",
            "<p id=target>Decoded</p><a name='%74arget' id=legacy>Literal anchor</a>",
            vec!["legacy"],
        ),
        (
            "t%C3%A9",
            "<p id=té>Unicode</p><p id=té>Duplicate</p>",
            vec!["té"],
        ),
        ("top", "<p id=sample>Top</p>", vec![]),
    ] {
        let mut tree =
            DocumentTree::parse(&format!("<!doctype html>{html}")).expect("parse test HTML");
        tree.set_document_urls(
            Some(crate::Url::parse(&format!("https://example.test/page#{fragment}")).unwrap()),
            crate::Url::parse("https://base.test/elsewhere/").unwrap(),
        );
        let selector = ferri_pdf_dom::Selector::parse(":target").unwrap();
        let selected: Vec<_> = tree
            .select(&selector)
            .filter_map(|element| element.attr("id"))
            .collect();
        assert_eq!(
            selected, expected,
            "HTML fragment resolution for {fragment:?}"
        );
    }
}

#[test]
fn radio_initial_state_respects_fragment_roots_and_empty_form_references() {
    let tree = DocumentTree::parse(
        "<!doctype html><form id=''><input id=outer name=x type=radio checked></form><input id=orphan form='' name=x type=radio checked><template><input id=first name=x type=radio checked><input id=last name=x type=radio checked></template>",
    ).expect("parse test HTML");
    for id in ["outer", "orphan"] {
        let selector = ferri_pdf_dom::Selector::parse(&format!("#{id}")).unwrap();
        assert!(
            tree.select(&selector).next().unwrap().initially_checked(),
            "empty form IDs cannot bind form='': {id}"
        );
    }
    let template_selector = ferri_pdf_dom::Selector::parse("template").unwrap();
    let template = tree.select(&template_selector).next().unwrap();
    let blank = ferri_pdf_dom::Selector::parse(":blank").unwrap();
    let selected: Vec<_> = template
        .template_contents()
        .unwrap()
        .inclusive_descendants(ferri_pdf_dom::TreeRelation::Selector)
        .filter_map(ferri_pdf_dom::Node::element)
        .filter(|element| element.matches(&blank))
        .filter_map(|element| element.attr("id"))
        .collect();
    assert_eq!(
        selected,
        ["first"],
        "each template document fragment has its own initial radio group"
    );
}
fn marked(selector: &str, html: &str) -> Vec<String> {
    marked_sources(&[&format!("{selector}{{color:#123456}}")], html)
}

#[test]
fn namespace_rules_distinguish_html_svg_and_attributes() {
    for (css, expected) in [
        (
            "@namespace h 'http://www.w3.org/1999/xhtml';h|p{color:#123456}",
            vec!["html", "inside"],
        ),
        (
            "@namespace 'http://www.w3.org/2000/svg';*{color:#123456}",
            vec!["svg", "plain"],
        ),
        (
            "@namespace m url(http://www.w3.org/1998/Math/MathML);m|mi{color:#123456}",
            vec!["math"],
        ),
        ("*|p{color:#123456}", vec!["html", "inside"]),
        ("|*{color:#123456}", vec![]),
        ("@namespace '';*{color:#123456}", vec![]),
        (
            "@namespace s 'http://www.w3.org/2000/svg';s|*[href]{color:#123456}",
            vec!["plain"],
        ),
        (
            "@namespace x 'http://www.w3.org/1999/xlink';[x|href]{color:#123456}",
            vec!["svg"],
        ),
        ("[*|href]{color:#123456}", vec!["svg", "plain"]),
        (
            "@namespace 'http://www.w3.org/2000/svg';[href]{color:#123456}",
            vec!["plain"],
        ),
        (
            "@namespace s 'http://invalid.example/one';@namespace s 'http://www.w3.org/2000/svg';s|g{color:#123456}",
            vec!["svg", "plain"],
        ),
        (
            "@namespace s 'http://www.w3.org/2000/svg';S|g{color:#123456}",
            vec![],
        ),
        ("#html, missing|g{color:#123456}", vec![]),
    ] {
        assert_eq!(
            marked_sources(&[css], ELEMENTS),
            expected,
            "namespace rule: {css}"
        );
    }
    assert_eq!(
        marked_sources(
            &[
                "@namespace s 'http://www.w3.org/2000/svg';s|g{color:red}",
                "s|g{color:#123456}"
            ],
            ELEMENTS
        ),
        Vec::<String>::new(),
        "prefix declarations cannot leak into a subsequent sheet"
    );
    assert_eq!(
        marked_sources(
            &[
                "@namespace 'http://www.w3.org/2000/svg';g{color:red}",
                "p{color:#123456}"
            ],
            ELEMENTS
        ),
        ["html", "inside"],
        "a default namespace cannot leak into the next stylesheet"
    );
}

#[test]
fn selector_lists_recover_by_their_defined_rules() {
    const HTML: &str = "<!doctype html><ul><li id=a class=item>A</li><li id=b>B</li><li id=c class=item>C</li></ul><div id=space> \n\t </div><div id=comment><!-- ignored --></div><div id=nbsp>&nbsp;</div>";
    for (selector, expected) in [
        ("li,:unknown", vec![]),
        (":is(li,:unknown)", vec!["a", "b", "c"]),
        (":where(li,:unknown)", vec!["a", "b", "c"]),
        ("li:not(.absent,:unknown)", vec![]),
        ("li:has(+ li,:unknown)", vec![]),
        ("li:has(+ li)", vec!["a", "b"]),
        ("ul:has(:has(li)) li", vec![]),
        ("li:nth-child(2 of .item)", vec!["c"]),
        ("li:nth-last-child(2 of :is(.item))", vec!["a"]),
        (".item:not(:first-child)", vec!["c"]),
        (":empty", vec!["space", "comment"]),
    ] {
        assert_eq!(
            marked(selector, HTML),
            expected,
            "selector list: {selector}"
        );
    }
    assert_eq!(
        marked_sources(&[":is(#a,.item){color:red}.item{color:#123456}"], HTML),
        Vec::<String>::new(),
        "is() retains the greatest argument specificity, including nonmatching alternatives"
    );
    assert_eq!(
        marked_sources(&[":where(#a){color:red}.item{color:#123456}"], HTML),
        ["a", "c"],
        "where() contributes zero specificity"
    );
}

#[test]
fn static_selector_states_match_initial_html() {
    for (html, selector, expected) in [
        (
            "<!doctype html><div lang=de-Latn-DE><p id=inherited>Text</p><p id=english lang=en>Text</p></div>",
            ":lang(de-DE)",
            vec!["inherited"],
        ),
        (
            "<!doctype html><p id=known lang=und>Text</p><p id=unknown lang=''>Text</p>",
            ":lang('*')",
            vec!["known"],
        ),
        (
            "<!doctype html><p id=known lang=und>Text</p><p id=unknown lang=''>Text</p>",
            ":lang('')",
            vec!["unknown"],
        ),
        (
            "<!doctype html><div dir=RTL><p id=inherited>Latin</p><p id=auto dir=auto>אב</p><p id=ltr dir=ltr>Latin</p></div>",
            ":dir(rtl)",
            vec!["inherited", "auto"],
        ),
        (
            "<!doctype html><p id=ltr dir=ltr style='direction:rtl'>Text</p>",
            ":dir(ltr)",
            vec!["ltr"],
        ),
        (
            "<!doctype html><p id=builtin>Built in</p><x-widget id=custom>Custom</x-widget><svg><x-widget id=foreign></x-widget></svg>",
            ":defined",
            vec!["builtin", "foreign"],
        ),
        (
            "<!doctype html><a id=link href='#target'>Link</a><a id=missing>Not a link</a><area id=area href='#target'>",
            ":any-link",
            vec!["link", "area"],
        ),
        (
            "<!doctype html><input id=text><input id=spaces value=' '><input id=unchecked type=checkbox><input id=checked type=checkbox checked><textarea id=textarea></textarea><input id=number type=number value=invalid><input id=range type=range><input id=color type=color><button id=button>Button</button>",
            ":blank",
            vec!["text", "unchecked", "textarea", "number"],
        ),
    ] {
        assert_eq!(
            marked(selector, html),
            expected,
            "initial state: {selector} in {html}"
        );
    }
    let html = "<!doctype html><p id=target interestfor=popup autofocus>Static</p><a id=link href='#target'>Link</a>";
    for selector in [
        ":hover",
        ":active",
        ":focus",
        ":focus-visible",
        ":focus-within",
        ":visited",
        ":interest-source",
        ":interest-target",
        ":current",
        ":past",
        ":future",
    ] {
        assert!(
            marked(selector, html).is_empty(),
            "no fabricated interaction/timeline state: {selector}"
        );
    }
}

#[test]
fn heading_selectors_share_the_computed_html_heading_level() {
    const HTML: &str = "<!doctype html><section headingoffset=2><h1 id=offset>Offset</h1><section headingreset><h2 id=reset>Reset</h2></section></section><h4 id=plain>Plain</h4>";
    assert_eq!(marked(":heading", HTML), ["offset", "reset", "plain"]);
    assert_eq!(marked(":heading(3,4)", HTML), ["offset", "plain"]);
    assert_eq!(marked(":heading(2)", HTML), ["reset"]);
    assert_eq!(
        marked_sources(&[":heading{color:#123456}h1{color:red}"], HTML),
        ["offset", "reset", "plain"],
        "heading has class specificity"
    );
    for invalid in [
        ":heading()",
        ":heading(1.0)",
        ":heading(1,)",
        ":heading(calc(1))",
    ] {
        assert!(
            marked(invalid, HTML).is_empty(),
            "invalid heading grammar: {invalid}"
        );
    }
}

#[test]
fn selector_support_queries_use_the_actual_selector_implementation() {
    for (selector, expected) in [
        ("p:lang(en)", true),
        ("li:nth-child(2 of .item)", true),
        ("::slotted(p)::before", true),
        ("x-host::part(label)::after", true),
        (":has-slotted", true),
        (":has-slotted(*)", false),
        (":heading(2,3)", true),
        ("missing|p", false),
        (":is(p,:unknown)", false),
        (":has(:has(p))", false),
    ] {
        let html = format!(
            "<!doctype html><style>{}p{{font-size:12pt}}@supports selector({selector}){{p{{font-size:19pt}}}}</style><p>Support</p>",
            common::PROFILE
        );
        let output = crate::render_html(&html, &local_options()).unwrap();
        assert_pt(
            output.layout.unwrap().pages[0][0].font_size,
            if expected { 19.0 } else { 12.0 },
        );
    }
}

#[test]
fn static_link_and_target_selectors_use_the_original_document_url() {
    let mut options = local_options();
    options.base = crate::Url::parse("https://example.test/docs/page.html#target")
        .unwrap()
        .into();
    let html = "<!doctype html><style>p,a{color:black}#target:target,#same:local-link,#near:local-link(1){color:#123456}a:visited{color:red}</style><p id=target>Target</p><p><a id=same href='page.html#other'>Same</a></p><p><a id=near href='other.html'>Near</a></p><p><a href='https://other.test/docs/page.html'>Other</a></p>";
    let trace = crate::render_html(html, &options).unwrap().layout.unwrap();
    let colors: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .map(|paragraph| paragraph.runs[0].color.rgb)
        .collect();
    assert_eq!(
        colors,
        [MARKED, MARKED, MARKED, [0, 0, 0]],
        "fragment targets and hierarchical local links use document context without inventing visit history"
    );
}
