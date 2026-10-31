use super::discarded;
use crate::style::{Degradation, declaration, grammar::Property};

#[test]
fn values_serialize_as_css() {
    for (property, source, expected) in [
        (
            Property::TextShadow,
            "1px 1px rgba(255,0,0,.5), 2px 2px #00f",
            "1px 1px rgba(255, 0, 0, 0.5), 2px 2px rgb(0, 0, 255)",
        ),
        (
            Property::TextShadow,
            "1px 1px oklch(70% 0.1 none / 50%), 2px 2px color(xyz .1 .2 .3), 0 0 lab(50 20 -30)",
            "1px 1px oklch(0.7 0.1 none / 0.5), 2px 2px color(xyz-d65 0.1 0.2 0.3), 0 0 lab(50 20 -30)",
        ),
        (
            Property::ClipPath,
            "polygon(0 0,100% 0,50% 100%)",
            "polygon(0 0, 100% 0, 50% 100%)",
        ),
        (
            Property::FontFeatureSettings,
            "'liga' 1,\"k\\\"rn\"",
            "\"liga\" 1, \"k\\\"rn\"",
        ),
        (
            Property::Cursor,
            "url(\"a b.png\") 2 3,pointer",
            "url(\"a b.png\") 2 3, pointer",
        ),
        (
            Property::WillChange,
            "transform,opacity",
            "transform, opacity",
        ),
        (
            Property::TransitionTimingFunction,
            "cubic-bezier(.1,.2,.3,.4),ease",
            "cubic-bezier(0.1, 0.2, 0.3, 0.4), ease",
        ),
    ] {
        let value = declaration::value(property, source, &mut 0)
            .unwrap()
            .unwrap_or_else(|| panic!("`{source}` is a valid {}", property.name()));
        assert_eq!(
            value.to_string(),
            expected,
            "{}: `{source}` must serialize as CSS, matched as {value:?}",
            property.name()
        );
    }
}

#[test]
fn unsupported_values_name_their_declaration_once() {
    for (css, declaration, reason) in [
        (
            "#target{scroll-snap-type:x mandatory}",
            "scroll-snap-type: x mandatory",
            "unsupported CSS",
        ),
        (
            "#target{scroll-initial-target:nearest}",
            "scroll-initial-target: nearest",
            "unsupported CSS",
        ),
        (
            "#target{width:calc-mix(--clock,10pt,30pt)}",
            "width: calc-mix(--clock,10pt,30pt)",
            "unsupported CSS: animation timeline interpolation is outside the static print profile",
        ),
    ] {
        assert_eq!(
            discarded(css),
            [Degradation::InvalidAtComputedValueTime {
                declaration: declaration.into(),
                reason: reason.into(),
            }],
            "the one report for `{css}`"
        );
    }
}
