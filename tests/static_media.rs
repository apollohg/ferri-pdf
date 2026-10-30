mod common;

/// Declarations of every property whose specification gives it no effect in static paged
/// media, with non-initial values and through their shorthands and legacy aliases.
const WITHOUT_STATIC_EFFECT: &str = "\
    animation: 2s steps(4, end) 1s infinite alternate both paused bounce, fade .5s;\
    animation-composition: add;\
    animation-range: entry 10% exit 90%;\
    animation-timeline: scroll(root block), view(x 10px), --main;\
    animation-trigger: --hover play pause;\
    transition: color .2s cubic-bezier(.4, 0, .2, 1), background-color 1s ease-in .5s allow-discrete;\
    interpolate-size: allow-keywords;\
    scroll-timeline: --main inline, --side;\
    view-timeline: --card x 10% 2em;\
    timeline-scope: --main, --card;\
    timeline-trigger-name: --enter;\
    timeline-trigger-source: view();\
    timeline-trigger-range-start: entry 20%;\
    timeline-trigger-range-end: cover 80%;\
    timeline-trigger-exit-range-start: normal;\
    timeline-trigger-exit-range-end: exit 50%;\
    trigger-scope: all;\
    view-transition-class: card hero;\
    scroll-behavior: smooth;\
    scroll-margin: 1em 2px;\
    scroll-padding: 10% auto 3pt;\
    scroll-snap-align: center start;\
    scroll-snap-stop: always;\
    scroll-snap-type: both proximity;\
    overscroll-behavior: contain none;\
    overflow-anchor: none;\
    scrollbar-color: red rgb(0 0 255);\
    scrollbar-width: thin;\
    cursor: url(cursor.png) 2 3, pointer;\
    caret: currentcolor manual block;\
    pointer-events: none;\
    user-select: all;\
    touch-action: pan-x pinch-zoom;\
    resize: both;\
    ime-mode: disabled;\
    interactivity: inert;\
    interest-delay: 1s 250ms;\
    text-size-adjust: 150%;\
    forced-color-adjust: none;\
    print-color-adjust: exact;\
    dynamic-range-limit: dynamic-range-limit-mix(standard 30%, no-limit 70%);\
    speak-as: spell-out digits;\
    reading-flow: flex-flow;\
    reading-order: 2;\
";

/// Each declaration the engine does not accept prints itself before the first paragraph.
fn acceptance_probes() -> String {
    WITHOUT_STATIC_EFFECT
        .split(';')
        .map(str::trim)
        .filter(|declaration| !declaration.is_empty())
        .map(|declaration| {
            format!("@supports not ({declaration}){{body::before{{content:{declaration:?}}}}}")
        })
        .collect()
}

fn page(declarations: &str, probes: &str) -> String {
    format!(
        "<!doctype html><style>{}{probes}html,p,li{{{declarations}}}</style>\
         <p>Static <b>media</b> text</p><ol><li>One<li>Two</ol><p style='{declarations}'>Inline</p>",
        common::PROFILE
    )
}

fn render(html: &str) -> ferri_pdf::RenderOutput {
    ferri_pdf::render_html(html, &common::local_options())
        .unwrap_or_else(|error| panic!("render `{html}`: {error}"))
}

#[test]
fn properties_without_static_media_effect_leave_the_pdf_unchanged() {
    let baseline = render(&page("", "")).pdf;
    assert_eq!(
        baseline,
        render(&page("", "")).pdf,
        "rendering must be deterministic for the comparison to mean anything"
    );
    let with = render(&page(WITHOUT_STATIC_EFFECT, &acceptance_probes()));
    let lines: Vec<String> = with
        .layout
        .expect("layout requested")
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.clone())
        .collect();
    assert!(
        with.pdf == baseline,
        "declarations without effect in static media must be accepted and change nothing; \
         {} PDF bytes with them, {} without; rendered lines (unaccepted declarations print first): {lines:#?}",
        with.pdf.len(),
        baseline.len()
    );
}

/// At-rules whose specifications give them no effect on a static rendering, including
/// `@starting-style` nested in a style rule and a `@keyframes` name only a string may spell.
const RULES_WITHOUT_STATIC_EFFECT: &str = "\
    @keyframes fade { from { opacity: 0 } 50%, to { opacity: 1; color: red } }\
    @keyframes \"none\" { to { margin-left: 2in } }\
    p { animation: fade 2s both; animation-delay: -1s }\
    @starting-style { p { color: red; font-size: 40pt } }\
    li { transition: color 1s; @starting-style { margin-top: 1in; color: blue } }\
    @view-transition { navigation: auto; types: slide forward }\
";

fn diagnostics(output: &ferri_pdf::RenderOutput) -> Vec<String> {
    output.diagnostics.iter().map(ToString::to_string).collect()
}

#[test]
fn at_rules_without_static_media_effect_are_accepted_and_change_nothing() {
    let baseline = render(&page("", ""));
    let with = render(&page("", RULES_WITHOUT_STATIC_EFFECT));
    assert!(
        with.diagnostics.is_empty(),
        "valid rules must not be reported: {:#?}",
        diagnostics(&with)
    );
    assert!(
        with.pdf == baseline.pdf,
        "keyframes, starting styles and view-transition rules must not change a static \
         rendering; {} PDF bytes with them, {} without",
        with.pdf.len(),
        baseline.pdf.len()
    );
}

#[test]
fn inert_at_rules_with_invalid_preludes_are_reported() {
    for (rule, name) in [
        ("@keyframes none { to { color: red } }", "@keyframes"),
        ("@keyframes initial { to { color: red } }", "@keyframes"),
        ("@keyframes two names { to { color: red } }", "@keyframes"),
        ("@keyframes 12 { to { color: red } }", "@keyframes"),
        ("@starting-style p { p { color: red } }", "@starting-style"),
        (
            "@view-transition auto { navigation: auto }",
            "@view-transition",
        ),
    ] {
        let reported = diagnostics(&render(&page("", rule)));
        assert!(
            reported.len() == 1 && reported[0].contains(name),
            "`{rule}` has an invalid prelude and must be reported once: {reported:#?}"
        );
    }
}
