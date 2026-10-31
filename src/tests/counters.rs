use super::common;
use crate::render_html;
use common::{PROFILE, local_options};

fn lines(css: &str, body: &str) -> Vec<String> {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("document counters must render through the common cascade")
    .layout
    .unwrap()
    .pages
    .into_iter()
    .flatten()
    .map(|line| line.text)
    .collect()
}

#[test]
fn counter_resets_on_siblings_replace_scope_and_preserve_ancestor_values() {
    let actual = lines(
        "body{counter-reset:chapter}h1{counter-increment:chapter;counter-reset:section}h2{counter-increment:section}h1::before{content:counter(chapter) '. '}h2::before{content:counter(chapter) '.' counter(section) ': '}",
        "<h1>First</h1><h2>One</h2><h2>Two</h2><h1>Second</h1><h2>Three</h2>",
    );
    assert_eq!(
        actual,
        [
            "1. First",
            "1.1: One",
            "1.2: Two",
            "2. Second",
            "2.1: Three"
        ]
    );
}

#[test]
fn nested_counter_representations_use_outer_to_inner_order() {
    let actual = lines(
        "body{counter-reset:part 1}section{counter-reset:part 3}section section{counter-reset:part 9}p{counter-increment:part}p::before{content:counters(part,'.',upper-alpha) ': '}",
        "<div><section><p>Outer</p><div><section><p>Inner</p></section></div><p>Outer again</p></section></div><p>Root</p>",
    );
    assert_eq!(
        actual,
        ["A.D: Outer", "A.D.J: Inner", "A.E: Outer again", "B: Root"]
    );
}

#[test]
fn counter_updates_apply_reset_then_increment_then_set_with_repeated_names() {
    let actual = lines(
        "body{counter-reset:n 8 n 10}p{counter-increment:n 2 n -1}p::before{content:counter(n) ': '}.set{counter-set:n -3 n 7}.reset{counter-reset:n 20 n 30;counter-increment:n 4}",
        "<p>First</p><p class=set>Set</p><p>After set</p><p class=reset>Reset</p><p>After reset</p>",
    );
    assert_eq!(
        actual,
        [
            "11: First",
            "7: Set",
            "8: After set",
            "34: Reset",
            "35: After reset"
        ]
    );
}

#[test]
fn counter_values_come_from_the_preceding_descendant_and_ignore_hidden_boxes() {
    let actual = lines(
        "body{counter-reset:n}p,span{counter-increment:n}p::before,span::before{content:counter(n) ': '}.hidden{display:none;counter-increment:n 100}",
        "<p>First <span>Inner</span></p><p class=hidden>Hidden</p><p>Last</p>",
    );
    assert_eq!(actual, ["1: First 2: Inner", "3: Last"]);
}

#[test]
fn pseudo_counter_instances_do_not_leak_beyond_their_parent_scope() {
    let actual = lines(
        "p::before{content:counter(n) '-'}p::after{counter-increment:n;content:'-' counter(n)}",
        "<p>First</p><p>Second</p>",
    );
    assert_eq!(actual, ["0-First-1", "0-Second-1"]);
}

#[test]
fn generated_counter_styles_reuse_marker_representation_without_suffixes() {
    let actual = lines(
        "body{counter-reset:n 26}p{counter-increment:n}p::before{content:counter(n,lower-alpha) '/' counter(n,upper-roman) '/' counter(n,decimal-leading-zero) '/' counter(n,none) ': '}",
        "<p>First</p><p>Second</p>",
    );
    assert_eq!(actual, ["aa/XXVII/27/: First", "ab/XXVIII/28/: Second"]);
}

#[test]
fn counter_text_does_not_create_an_unnecessary_first_page_geometry_cycle() {
    assert_eq!(
        lines(
            "body{counter-reset:n calc(100vw / 1px)}p::before{content:counter(n) ': '}",
            "<p>Body</p>"
        ),
        ["384: Body"]
    );
}

#[test]
fn boxless_counter_declarations_do_not_change_visible_descendant_counters() {
    assert_eq!(
        lines(
            "body{counter-reset:n 4}section{display:contents;counter-reset:n 100;counter-increment:n 9;counter-set:n 20}p{counter-increment:n}p::before{content:counter(n) ': '}",
            "<section><p>First</p></section><p>Second</p>"
        ),
        ["5: First", "6: Second"]
    );
}

#[test]
fn list_markers_and_generated_content_observe_the_same_counter() {
    assert_eq!(
        lines(
            "li{counter-increment:list-item 2}li::before{content:counter(list-item) ': '}",
            "<ol><li>First</li><li>Second</li></ol>"
        ),
        ["2. ", "2: First", "4. ", "4: Second"]
    );
}

#[test]
fn bookmark_labels_capture_counter_values_at_the_heading() {
    let html = format!(
        "<style>{PROFILE}body{{counter-reset:n}}h1{{counter-increment:n;bookmark-label:counter(n) ' ' content(text)}}</style><h1>First</h1><h1>Second</h1>"
    );
    let document = crate::document::StyledDocument::parse(
        &html,
        &local_options().base,
        &[],
        &crate::resources::FileResolver::default(),
    )
    .expect("counter labels must retain heading snapshots");
    assert_eq!(
        document
            .navigation()
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.label.as_str())
            .collect::<Vec<_>>(),
        ["1 First", "2 Second"]
    );
}

#[test]
fn reversed_counter_initial_values_include_updates_in_their_own_scope() {
    assert_eq!(
        lines(
            "body{counter-reset:reversed(n)}p{counter-increment:n -2}p::before{content:counter(n) ': '}",
            "<p>First</p><p>Second</p><p>Third</p>"
        ),
        ["6: First", "4: Second", "2: Third"]
    );
}

#[test]
fn first_generated_inline_counter_does_not_depend_on_the_page_it_selects() {
    let actual = lines(
        "@page cover{size:500pt 600pt}body{counter-reset:n calc(100vw / 1px)}span::before{content:counter(n)}h1{page:cover}",
        "<span></span><h1>Cover</h1>",
    );
    assert_eq!(actual, ["384", "Cover"]);
}

#[test]
fn html_list_hints_share_css_state_and_yield_to_authored_declarations() {
    assert_eq!(
        lines(
            "li::before{content:counter(list-item) ': '}.override{counter-reset:list-item 9}.override li{counter-set:list-item 4}",
            "<ol start=' +5suffix'><li>Start</li><li value='-2suffix'>Set</li><li>Next</li></ol><ol start=80 class=override><li value=60>Author</li></ol>"
        ),
        [
            "5. ",
            "5: Start",
            "-2. ",
            "-2: Set",
            "-1. ",
            "-1: Next",
            "4. ",
            "4: Author"
        ]
    );
}

#[test]
fn implicit_list_item_increment_is_replaced_only_when_named_explicitly() {
    assert_eq!(
        lines(
            "body{counter-reset:n}li{counter-increment:n}li::before{content:counter(list-item) '/' counter(n) ': '}.fixed{counter-increment:list-item 0}",
            "<ol start=3><li>First</li><li class=fixed>Fixed</li><li>Last</li></ol>"
        ),
        ["3. ", "3/1: First", "3. ", "3/1: Fixed", "4. ", "4/2: Last"]
    );
}

#[test]
fn counter_updates_clamp_at_the_shared_numeric_bounds_without_wrapping() {
    assert_eq!(
        lines(
            "@page{size:1000pt 1000pt}li::before{content:counter(list-item) ': '}.down li{counter-increment:list-item -1}",
            &format!(
                "<ol><li value='{}'>Maximum</li><li>Clamped</li></ol><ol class=down><li value='{}'>Minimum</li><li>Clamped</li></ol>",
                i128::MAX,
                i128::MIN
            )
        ),
        [
            format!("{}. ", i128::MAX),
            format!("{}: Maximum", i128::MAX),
            format!("{}. ", i128::MAX),
            format!("{}: Clamped", i128::MAX),
            format!("{}. ", i128::MIN),
            format!("{}: Minimum", i128::MIN),
            format!("{}. ", i128::MIN),
            format!("{}: Clamped", i128::MIN)
        ]
    );
}

#[test]
fn reversed_html_lists_share_markers_hints_and_counter_text() {
    assert_eq!(
        lines(
            "li::before{content:counter(list-item) ': '}",
            "<ol reversed><li>A</li><li>B</li><li>C</li></ol><ol reversed start=7><li>D</li><li>E</li></ol><ol reversed><li>Before set</li><li value=5>Set</li><li>After set</li></ol>"
        ),
        [
            "3. ",
            "3: A",
            "2. ",
            "2: B",
            "1. ",
            "1: C",
            "7. ",
            "7: D",
            "6. ",
            "6: E",
            "6. ",
            "6: Before set",
            "5. ",
            "5: Set",
            "4. ",
            "4: After set"
        ]
    );
}

#[test]
fn reversed_initial_values_stop_at_sets_and_exclude_nested_instances() {
    assert_eq!(
        lines(
            "body{counter-reset:reversed(n)}section{counter-reset:reversed(n)}p{counter-increment:n -2}p::before{content:counter(n) ': '}.set{counter-set:n 10}.hidden{display:none}",
            "<p>Outer</p><div><section><p>Inner</p><p>Inner last</p></section></div><p class=hidden>Hidden</p><p class=set>Set</p><p>After set</p>"
        ),
        [
            "12: Outer",
            "4: Inner",
            "2: Inner last",
            "10: Set",
            "8: After set"
        ]
    );
}

#[test]
fn first_page_counter_separator_depends_on_scope_without_reversed_values() {
    assert_eq!(
        lines(
            "@page cover{size:500pt 600pt}body{counter-reset:reversed(n)}span{counter-reset:reversed(n)}span::before{content:counters(n,'x',none)}h1{page:cover}",
            "<span></span><h1>Cover</h1>"
        ),
        ["x", "Cover"]
    );
}

#[test]
fn first_page_counter_separator_does_not_need_viewport_dependent_values() {
    assert_eq!(
        lines(
            "@page cover{size:500pt 600pt}body{counter-reset:n calc(100vw / 1px)}span{counter-reset:n}span::before{content:counters(n,'x',none)}h1{page:cover}",
            "<span></span><h1>Cover</h1>"
        ),
        ["x", "Cover"]
    );
}

#[test]
fn first_page_counter_scope_projection_preserves_cascade_and_substitution() {
    for reset in [
        "inherit",
        "var(--reset)",
        "toggle(n 1, n 2)",
        "attr(data-reset <custom-ident>)",
    ] {
        assert_eq!(
            lines(
                &format!(
                    "@page cover{{size:500pt 600pt}}body{{--reset:n calc(100vw / 1px);counter-reset:var(--reset)}}span{{counter-reset:{reset}}}span::before{{content:counters(n,'x',none)}}h1{{page:cover}}"
                ),
                "<span data-reset=n></span><h1>Cover</h1>"
            ),
            ["x", "Cover"],
            "scope projection must preserve {reset} without evaluating irrelevant counter numbers"
        );
    }
}

#[test]
fn reversed_counter_scan_ignores_suppressed_table_column_descendants() {
    assert_eq!(
        lines(
            "body{counter-reset:reversed(n)}p{counter-increment:n -1}p::before{content:counter(n) ': '}.column{display:table-column}.group{display:table-column-group}",
            "<p>First</p><div class=column><p>Suppressed column child</p></div><div class=group><p>Suppressed group child</p></div><p>Last</p>"
        ),
        ["2: First", "1: Last"]
    );
}

#[test]
fn reversed_counter_scan_normalizes_replaced_list_item_display() {
    assert_eq!(
        lines(
            "body{counter-reset:reversed(list-item)}p,svg{display:list-item;list-style-type:decimal}p::before{content:counter(list-item) ': '}",
            "<p>First</p><svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'></svg><p>Last</p>"
        ),
        ["2. ", "2: First", "\u{fffc}", "1. ", "1: Last"]
    );
}

#[test]
fn reversed_initial_values_cancel_offsets_before_clamping_at_numeric_bounds() {
    assert_eq!(
        lines(
            "@page{size:1000pt 1000pt}.upper li{counter-increment:list-item -2}.upper .set{counter-increment:list-item 1}.lower li{counter-increment:list-item 2}.lower .set{counter-increment:list-item -1}",
            &format!(
                "<ol reversed class=upper><li>Upper</li><li class=set value='{}'>Set</li></ol><ol reversed class=lower><li>Lower</li><li class=set value='{}'>Set</li></ol>",
                i128::MAX,
                i128::MIN
            )
        ),
        [
            format!("{}. ", i128::MAX - 2),
            "Upper".into(),
            format!("{}. ", i128::MAX),
            "Set".into(),
            format!("{}. ", i128::MIN + 2),
            "Lower".into(),
            format!("{}. ", i128::MIN),
            "Set".into()
        ]
    );
}

#[test]
fn counters_past_their_work_budget_stop_updating_instead_of_failing() {
    // Each increment finds its counter among the counters in scope, so incrementing the
    // outermost of many counters on many elements exceeds the counter work budget.
    const COUNTERS: usize = 5_000;
    const ELEMENTS: usize = 4_000;
    let resets = (0..COUNTERS)
        .map(|index| format!("c{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        "<style>body{{counter-reset:{resets}}}span{{counter-increment:c0}}</style><p>{}<b>Last</b></p>",
        "<span>x</span>".repeat(ELEMENTS)
    );
    let output = render_html(&html, &local_options())
        .expect("counters past the counter work budget must render");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            crate::RenderDiagnostic::FormattingLimit { resource, limit, .. }
                if *resource == crate::document::counters::COUNTER_WORK_RESOURCE
                    && *limit == crate::document::counters::MAX_COUNTER_WORK
        )),
        "the exhausted counter work is reported: {:?}",
        output.diagnostics
    );
    assert!(
        output
            .layout
            .expect("layout requested")
            .pages
            .iter()
            .flatten()
            .any(|line| line.text.contains("Last")),
        "the content past the budget still renders"
    );
}
