use crate::dom::{DocumentTree, Selector};
use crate::paint::{Easing, LengthPercentage};
use crate::resources::FileResolver;
use crate::style::{
    Degradation, Style, StyleError, Stylesheet, StylesheetSource,
    calc::Kind,
    grammar::Property,
    inert::{Computed, InertValues},
};

const DOCUMENT: &str = "<div id=parent><span id=child>Text</span></div>";

fn styles(css: &str) -> Result<(Style, Style), StyleError> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])?;
    let html = DocumentTree::parse(DOCUMENT).expect("parse test HTML");
    let select = |id: &str| html.select(&Selector::parse(id).unwrap()).next().unwrap();
    let parent = sheet.computed_with_resolver(
        select("#parent"),
        &Style::default(),
        &FileResolver::default(),
    )?;
    let child =
        sheet.computed_with_resolver(select("#child"), &parent, &FileResolver::default())?;
    Ok((parent, child))
}

fn inert(css: &str) -> InertValues {
    styles(css)
        .unwrap_or_else(|error| panic!("`{css}` must compute: {error}"))
        .0
        .inert
}

#[test]
fn values_inherit_as_their_specifications_define() {
    // Each specification's property table: CSS UI 4, CSS Size Adjust 1, CSS Speech 1,
    // CSS Color Adjust 1, CSS Display 4, CSS Transitions 1 and CSS Scroll Snap 1.
    for (declaration, property, inherited) in [
        ("cursor:pointer", Property::Cursor, true),
        ("caret-color:red", Property::CaretColor, true),
        ("pointer-events:none", Property::PointerEvents, true),
        ("interactivity:inert", Property::Interactivity, true),
        ("text-size-adjust:50%", Property::TextSizeAdjust, true),
        ("speak-as:spell-out", Property::SpeakAs, true),
        ("print-color-adjust:exact", Property::PrintColorAdjust, true),
        ("user-select:none", Property::UserSelect, false),
        ("resize:both", Property::Resize, false),
        ("reading-order:2", Property::ReadingOrder, false),
        (
            "transition-duration:1s",
            Property::TransitionDuration,
            false,
        ),
        (
            "scroll-snap-points-x:repeat(10%)",
            Property::ScrollSnapPointsX,
            false,
        ),
        (
            "scroll-snap-coordinate:left top,50% 1pt",
            Property::ScrollSnapCoordinate,
            false,
        ),
        (
            "scroll-snap-destination:right",
            Property::ScrollSnapDestination,
            false,
        ),
        (
            "scroll-snap-type-y:proximity",
            Property::ScrollSnapTypeY,
            false,
        ),
    ] {
        let (parent, child) = styles(&format!("#parent{{{declaration}}}"))
            .unwrap_or_else(|error| panic!("`{declaration}` must compute: {error}"));
        assert!(
            parent.inert.get(property).is_some(),
            "`{declaration}` must record a non-initial computed value, got {:?}",
            parent.inert
        );
        assert_eq!(
            child.inert.get(property).is_some(),
            inherited,
            "`{declaration}` is {}inherited, but the child computed {:?}",
            if inherited { "" } else { "not " },
            child.inert
        );
    }
}

#[test]
fn computed_values_resolve_lengths_and_math_on_the_declaring_element() {
    let (parent, child) = styles(
        "#parent{font-size:10pt;view-timeline-inset:2em;transition-duration:calc(1s + 500ms),2s;reading-order:calc(2.6)}\
         #child{font-size:20pt;view-timeline-inset:inherit;transition-duration:inherit;reading-order:inherit}",
    )
    .unwrap();
    assert_eq!(
        parent.inert.get(Property::ViewTimelineInset),
        Some(&Computed::Group(vec![Computed::Group(vec![
            Computed::LengthPercentage(LengthPercentage::Length(20.0))
        ])])),
        "2em against the parent's 10pt font is 20pt"
    );
    assert_eq!(
        child.inert.get(Property::ViewTimelineInset),
        parent.inert.get(Property::ViewTimelineInset),
        "`inherit` copies the computed 20pt instead of resolving 2em against the child's font"
    );
    assert_eq!(
        parent.inert.get(Property::TransitionDuration),
        Some(&Computed::Group(vec![
            Computed::Dimension(Kind::Time, 1.5),
            Computed::Dimension(Kind::Time, 2.0)
        ])),
        "a time calculation computes to seconds"
    );
    assert_eq!(
        child.inert.get(Property::ReadingOrder),
        Some(&Computed::Integer(3)),
        "an <integer> calculation rounds to the nearest integer (CSS Values 4 §10.9)"
    );
}

#[test]
fn easing_functions_compute_to_their_curves() {
    let values = inert(
        "#parent{animation-timing-function:ease-in,steps(calc(1.6),jump-none),cubic-bezier(.1,.2,.3,.4)}",
    );
    let Some(Computed::Group(curves)) = values.get(Property::AnimationTimingFunction) else {
        panic!("a list of computed easing functions, got {values:?}");
    };
    assert!(
        matches!(
            curves.as_slice(),
            [
                Computed::Easing(Easing::CubicBezier([0.42, 0.0, 1.0, 1.0])),
                Computed::Easing(Easing::Steps { count: 2, .. }),
                Computed::Easing(Easing::CubicBezier([0.1, 0.2, 0.3, 0.4])),
            ]
        ),
        "ease-in, two rounded jump-none steps and the written curve: {curves:?}"
    );
}

#[test]
fn shorthands_set_the_same_values_as_their_longhands() {
    for (shorthand, longhands) in [
        (
            "transition:color .2s ease-in 1s,background-color 1s allow-discrete",
            "transition-property:color,background-color;transition-duration:.2s,1s;\
             transition-timing-function:ease-in,ease;transition-delay:1s,0s;\
             transition-behavior:normal,allow-discrete",
        ),
        (
            "animation-range:entry;animation:a 1s 2s infinite,b 3s reverse",
            "animation-name:a,b;animation-duration:1s,3s;animation-delay:2s,0s;\
             animation-iteration-count:infinite,1;animation-direction:normal,reverse;\
             animation-timing-function:ease,ease;animation-fill-mode:none,none;\
             animation-play-state:running,running;animation-timeline:auto,auto",
        ),
        (
            "animation-range:entry 10%,20% exit,cover",
            "animation-range-start:entry 10%,20%,cover;animation-range-end:entry 100%,exit,cover 100%",
        ),
        (
            "timeline-trigger:--a view() cover 10% / contain,--b auto 5pt 6pt",
            "timeline-trigger-name:--a,--b;timeline-trigger-source:view(),auto;\
             timeline-trigger-range-start:cover 10%,5pt;timeline-trigger-range-end:cover 100%,6pt;\
             timeline-trigger-exit-range-start:contain,auto;\
             timeline-trigger-exit-range-end:contain 100%,auto",
        ),
        (
            "timeline-trigger-range:entry,1pt 2pt;timeline-trigger-exit-range:exit 5%",
            "timeline-trigger-range-start:entry,1pt;timeline-trigger-range-end:entry 100%,2pt;\
             timeline-trigger-exit-range-start:exit 5%;timeline-trigger-exit-range-end:exit 100%",
        ),
        (
            "scroll-timeline:--a,--b x",
            "scroll-timeline-name:--a,--b;scroll-timeline-axis:block,x",
        ),
        (
            "view-timeline:--a 1pt x,--b",
            "view-timeline-name:--a,--b;view-timeline-axis:x,block;view-timeline-inset:1pt,auto",
        ),
        (
            "overscroll-behavior:contain",
            "overscroll-behavior-x:contain;overscroll-behavior-y:contain",
        ),
        (
            "interest-delay:2s",
            "interest-delay-start:2s;interest-delay-end:2s",
        ),
        (
            "caret:red block",
            "caret-color:red;caret-shape:block;caret-animation:auto",
        ),
    ] {
        let expanded = inert(&format!("#parent{{{shorthand}}}"));
        assert_ne!(
            expanded,
            InertValues::default(),
            "`{shorthand}` must set non-initial values"
        );
        assert_eq!(
            expanded,
            inert(&format!("#parent{{{longhands}}}")),
            "`{shorthand}` must compute as `{longhands}`"
        );
    }
}

#[test]
fn authored_overlay_loses_to_the_user_agent_rule() {
    for declaration in ["overlay:auto", "overlay:auto !important"] {
        assert_eq!(
            inert(&format!("#parent{{{declaration}}}")).get(Property::Overlay),
            None,
            "CSS Position 4 §3.4: `* {{ overlay: none !important }}` must override `{declaration}`"
        );
    }
}

#[test]
fn transition_lists_exclude_none_beside_other_items() {
    for invalid in ["transition:none,color 1s", "transition-property:color,none"] {
        assert_eq!(
            inert(&format!("#parent{{transition-duration:3s;{invalid}}}")),
            inert("#parent{transition-duration:3s}"),
            "CSS Transitions 1 §2.1 makes `{invalid}` invalid, so it is dropped"
        );
    }
    assert!(
        inert("#parent{transition:none}")
            .get(Property::TransitionProperty)
            .is_some(),
        "a single `none` is valid and differs from the initial `all`"
    );
}

#[test]
fn mandatory_scroll_snapping_remains_unsupported() {
    assert!(
        inert("#parent{scroll-snap-type:both proximity}")
            .get(Property::ScrollSnapType)
            .is_some(),
        "proximity snapping only permits snapping, so it computes without effect"
    );
    for (declaration, expected) in [
        (
            "scroll-snap-type:x mandatory",
            "scroll-snap-type: x mandatory",
        ),
        (
            "scroll-snap-type-x:mandatory",
            "scroll-snap-type-x: mandatory",
        ),
        (
            "scroll-snap-type-y:mandatory",
            "scroll-snap-type-y: mandatory",
        ),
    ] {
        assert_eq!(
            super::discarded(&format!("#target{{{declaration}}}")),
            [Degradation::InvalidAtComputedValueTime {
                declaration: expected.into(),
                reason: "unsupported CSS".into(),
            }],
            "mandatory snapping can move the printed scroll position, so it computes as unset"
        );
    }
}
