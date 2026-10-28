use crate::fonts::{FeatureValueType, FontFeature};
use crate::style::{FontKerning, FontVariantAlternates, FontVariantCaps, Style};
use crate::text::features::{
    CapsPlan, FaceFeatures, alternate_features, caps_plan, position_feature, run_features,
};
use std::collections::BTreeMap;
use std::sync::Arc;

const SMALL_CAPS: &[FontFeature] = &[FontFeature::on(*b"smcp")];
/// Native feature tags, and whether lowercase and uppercase letters are synthesized.
type ExpectedPlan = (&'static [&'static str], bool, bool);

fn tag(feature: FontFeature) -> String {
    String::from_utf8_lossy(&feature.tag).into_owned()
}

fn listed(features: &[FontFeature]) -> Vec<(String, u32)> {
    features
        .iter()
        .map(|feature| (tag(*feature), feature.value))
        .collect()
}

/// The value the shaper applies for each tag: the last setting in the list.
fn effective(features: &[FontFeature]) -> BTreeMap<String, u32> {
    features
        .iter()
        .map(|feature| (tag(*feature), feature.value))
        .collect()
}

#[test]
fn caps_plans_fall_back_through_the_features_a_font_has() {
    let fonts: [(&str, &[&[u8; 4]]); 4] = [
        (
            "everything",
            &[b"smcp", b"c2sc", b"pcap", b"c2pc", b"unic", b"titl"],
        ),
        ("small capitals only", &[b"smcp", b"c2sc"]),
        ("lowercase small capitals only", &[b"smcp"]),
        ("none", &[]),
    ];
    let expected: [(FontVariantCaps, [ExpectedPlan; 4]); 7] = [
        (
            FontVariantCaps::Normal,
            [
                (&[], false, false),
                (&[], false, false),
                (&[], false, false),
                (&[], false, false),
            ],
        ),
        (
            FontVariantCaps::SmallCaps,
            [
                (&["smcp"], false, false),
                (&["smcp"], false, false),
                (&["smcp"], false, false),
                (&[], true, false),
            ],
        ),
        (
            FontVariantCaps::AllSmallCaps,
            [
                (&["c2sc", "smcp"], false, false),
                (&["c2sc", "smcp"], false, false),
                (&[], true, true),
                (&[], true, true),
            ],
        ),
        (
            FontVariantCaps::PetiteCaps,
            [
                (&["pcap"], false, false),
                (&["smcp"], false, false),
                (&["smcp"], false, false),
                (&[], true, false),
            ],
        ),
        (
            FontVariantCaps::AllPetiteCaps,
            [
                (&["c2pc", "pcap"], false, false),
                (&["c2sc", "smcp"], false, false),
                (&[], true, true),
                (&[], true, true),
            ],
        ),
        (
            FontVariantCaps::Unicase,
            [
                (&["unic"], false, false),
                (&[], false, true),
                (&[], false, true),
                (&[], false, true),
            ],
        ),
        (
            FontVariantCaps::TitlingCaps,
            [
                (&["titl"], false, false),
                (&[], false, false),
                (&[], false, false),
                (&[], false, false),
            ],
        ),
    ];
    for (caps, plans) in expected {
        for ((font, tags), (features, lower, upper)) in fonts.iter().zip(plans) {
            let has = |candidate: [u8; 4]| tags.contains(&&candidate);
            let plan = caps_plan(caps, has, true);
            assert_eq!(
                (
                    plan.features.iter().copied().map(tag).collect::<Vec<_>>(),
                    plan.synthetic_lower,
                    plan.synthetic_upper
                ),
                (
                    features.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    lower,
                    upper
                ),
                "{caps:?} with a font that has {font}"
            );
            let forbidden = caps_plan(caps, has, false);
            assert_eq!(
                forbidden,
                CapsPlan {
                    synthetic_lower: false,
                    synthetic_upper: false,
                    ..plan
                },
                "{caps:?} with {font}: font-synthesis-small-caps:none keeps native features and never synthesizes"
            );
        }
    }
}

#[test]
fn run_features_follow_the_css_fonts_precedence_order() {
    let mut style = Style::default();
    let features = &mut style.font_features;
    features.kerning = FontKerning::None;
    features.ligatures = Arc::from([FontFeature::off(*b"dlig"), FontFeature::on(*b"calt")]);
    features.numeric = Arc::from([FontFeature::on(*b"tnum")]);
    features.east_asian = Arc::from([FontFeature::on(*b"ruby")]);
    features.settings = Arc::from([
        FontFeature::on(*b"dlig"),
        FontFeature {
            tag: *b"kern",
            value: 1,
        },
    ]);
    let descriptor = [
        FontFeature::on(*b"dlig"),
        FontFeature::on(*b"onum"),
        FontFeature::off(*b"tnum"),
    ];
    let alternates = [FontFeature {
        tag: *b"salt",
        value: 3,
    }];
    let face = FaceFeatures {
        descriptor: &descriptor,
        alternates: &alternates,
        caps: SMALL_CAPS,
        position: position_feature(crate::style::FontVariantPosition::Super),
    };
    let list = run_features(&style, face, true);
    assert_eq!(
        listed(&list),
        [
            ("dlig", 1),
            ("onum", 1),
            ("tnum", 0),
            ("kern", 0),
            ("dlig", 0),
            ("calt", 1),
            ("sups", 1),
            ("smcp", 1),
            ("tnum", 1),
            ("salt", 3),
            ("ruby", 1),
            ("liga", 0),
            ("clig", 0),
            ("dlig", 1),
            ("kern", 1),
        ]
        .map(|(tag, value)| (tag.to_owned(), value)),
        "descriptor, then font-kerning and the font-variant subproperties, then tracking, then font-feature-settings"
    );
    let applied = effective(&list);
    assert_eq!(
        applied["dlig"], 1,
        "font-feature-settings re-enables what font-variant-ligatures disables (§7.3)"
    );
    assert_eq!(
        applied["tnum"], 1,
        "font-variant-numeric overrides the @font-face descriptor"
    );
    assert_eq!(
        applied["kern"], 1,
        "font-feature-settings overrides font-kerning"
    );
    assert_eq!(
        applied["liga"], 0,
        "tracked text suppresses common ligatures after font-variant"
    );
    let untracked = effective(&run_features(&style, face, false));
    assert!(
        !untracked.contains_key("liga") && !untracked.contains_key("clig"),
        "only tracking disables common ligatures: {untracked:?}"
    );
    let initial = run_features(&Style::default(), FaceFeatures::default(), false);
    assert!(
        initial.is_empty(),
        "initial values leave the shaper's default features alone: {initial:?}"
    );
    style.font_features.kerning = FontKerning::Normal;
    style.font_features.settings = Arc::from([]);
    assert_eq!(
        effective(&run_features(&style, FaceFeatures::default(), false))["kern"],
        1
    );
}

#[test]
fn alternates_resolve_only_names_the_family_defines() {
    let definitions: [(FeatureValueType, &str, &[u32]); 9] = [
        (FeatureValueType::Stylistic, "alt", &[2]),
        (FeatureValueType::Styleset, "sets", &[1, 12, 0, 100]),
        (FeatureValueType::Styleset, "more", &[3]),
        (FeatureValueType::CharacterVariant, "single", &[7]),
        (FeatureValueType::CharacterVariant, "valued", &[20, 3]),
        (FeatureValueType::CharacterVariant, "too-high", &[120, 1]),
        (FeatureValueType::Swash, "flowing", &[2]),
        (FeatureValueType::Ornaments, "bullet", &[4]),
        (FeatureValueType::Annotation, "circled", &[1]),
    ];
    let lookup = |kind, name: &str| {
        definitions
            .iter()
            .find(|(candidate, defined, _)| *candidate == kind && *defined == name)
            .map(|(_, _, values)| *values)
    };
    let alternates = FontVariantAlternates {
        historical_forms: true,
        stylistic: Some("alt".into()),
        styleset: vec!["sets".into(), "undefined".into(), "more".into()],
        character_variant: vec!["single".into(), "valued".into(), "too-high".into()],
        swash: Some("flowing".into()),
        ornaments: Some("bullet".into()),
        annotation: Some("circled".into()),
    };
    assert_eq!(
        listed(&alternate_features(&alternates, lookup)),
        [
            ("salt", 2),
            ("hist", 1),
            ("ss01", 1),
            ("ss12", 1),
            ("ss03", 1),
            ("cv07", 1),
            ("cv20", 3),
            ("swsh", 2),
            ("cswh", 2),
            ("ornm", 4),
            ("nalt", 1),
        ]
        .map(|(tag, value)| (tag.to_owned(), value)),
        "indices 0 and above 99 enable no feature; undefined names are ignored (§6.8, §6.9.2)"
    );
    let undefined = FontVariantAlternates {
        swash: Some("flowing".into()),
        styleset: vec!["sets".into()],
        ..Default::default()
    };
    assert!(
        alternate_features(&undefined, |_, _| None).is_empty(),
        "a family without definitions selects no alternates"
    );
}
