//! The OpenType features of a text run in CSS Fonts 4 §7.2 precedence order, with the caps and
//! position fallbacks of §6.5–§6.6. Later settings of a tag replace earlier ones.
use crate::fonts::{FeatureValueType, FontFeature};
use crate::style::{
    FontKerning, FontVariantAlternates, FontVariantCaps, FontVariantPosition, Style,
};
use icu_casemap::{CaseMapper, ContextCase};
use icu_properties::{CodePointSetData, props::Cased};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub(super) const KERNING: [u8; 4] = *b"kern";
const LIGATURES: [u8; 4] = *b"liga";
const CONTEXTUAL_LIGATURES: [u8; 4] = *b"clig";
const SMALL_CAPITALS: [u8; 4] = *b"smcp";
const CAPITALS_TO_SMALL: [u8; 4] = *b"c2sc";
const PETITE_CAPITALS: [u8; 4] = *b"pcap";
const CAPITALS_TO_PETITE: [u8; 4] = *b"c2pc";
const UNICASE: [u8; 4] = *b"unic";
const TITLING: [u8; 4] = *b"titl";
const SUBSCRIPT: [u8; 4] = *b"subs";
const SUPERSCRIPT: [u8; 4] = *b"sups";
const HISTORICAL_FORMS: [u8; 4] = *b"hist";
const STYLISTIC_ALTERNATES: [u8; 4] = *b"salt";
const SWASH: [u8; 4] = *b"swsh";
const CONTEXTUAL_SWASH: [u8; 4] = *b"cswh";
const ORNAMENTS: [u8; 4] = *b"ornm";
const ANNOTATION: [u8; 4] = *b"nalt";
/// `ss01`–`ss99` and `cv01`–`cv99` (§6.9.2); other indices enable no feature.
const STYLESET_PREFIX: [u8; 2] = *b"ss";
const CHARACTER_VARIANT_PREFIX: [u8; 2] = *b"cv";
const NUMBERED_FEATURES: std::ops::RangeInclusive<u32> = 1..=99;
const DECIMAL: u32 = 10;

const SMALL_CAPS: &[FontFeature] = &[FontFeature::on(SMALL_CAPITALS)];
const ALL_SMALL_CAPS: &[FontFeature] = &[
    FontFeature::on(CAPITALS_TO_SMALL),
    FontFeature::on(SMALL_CAPITALS),
];
const PETITE_CAPS: &[FontFeature] = &[FontFeature::on(PETITE_CAPITALS)];
const ALL_PETITE_CAPS: &[FontFeature] = &[
    FontFeature::on(CAPITALS_TO_PETITE),
    FontFeature::on(PETITE_CAPITALS),
];
const UNICASE_CAPS: &[FontFeature] = &[FontFeature::on(UNICASE)];
const TITLING_CAPS: &[FontFeature] = &[FontFeature::on(TITLING)];
/// Common ligatures are optional and do not apply across tracked letters (CSS Text 3 §8.2).
const TRACKED_LIGATURES: [FontFeature; 2] = [
    FontFeature::off(LIGATURES),
    FontFeature::off(CONTEXTUAL_LIGATURES),
];

/// UA synthesis policy; CSS Fonts leaves the reduction factor to implementations.
pub(crate) const SMALL_CAPS_SCALE: f32 = 0.8;

/// How a face renders `font-variant-caps`: its own features, or capitals scaled from the
/// uppercase forms of lowercase or uppercase letters (§6.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapsPlan {
    pub features: &'static [FontFeature],
    pub synthetic_lower: bool,
    pub synthetic_upper: bool,
}
impl CapsPlan {
    const NATIVE_NONE: Self = Self::native(&[]);
    const fn native(features: &'static [FontFeature]) -> Self {
        Self {
            features,
            synthetic_lower: false,
            synthetic_upper: false,
        }
    }
    pub(crate) fn synthetic(self) -> bool {
        self.synthetic_lower || self.synthetic_upper
    }
    /// `range` of `text` in logical order, split into the grapheme clusters this plan
    /// synthesizes as scaled capitals (`true`) and those it shapes as they are.
    pub(crate) fn pieces(self, text: &str, range: Range<usize>) -> Vec<(Range<usize>, bool)> {
        let cased = CodePointSetData::new::<Cased>();
        let mut pieces: Vec<(Range<usize>, bool)> = Vec::new();
        for (offset, cluster) in text[range.clone()].grapheme_indices(true) {
            let start = range.start + offset;
            let small = cluster.chars().any(|ch| {
                if ch.is_lowercase() {
                    self.synthetic_lower
                } else {
                    self.synthetic_upper && cased.contains(ch)
                }
            });
            if let Some((previous, same)) = pieces.last_mut()
                && *same == small
            {
                previous.end = start + cluster.len();
            } else {
                pieces.push((start..start + cluster.len(), small));
            }
        }
        pieces
    }
}

/// The uppercase forms synthesized capitals are shaped from (§6.6): each grapheme cluster of
/// `range` of `paragraph`, cased for the BCP 47 `language` in the paragraph's context, with
/// its offset in the range.
pub(crate) fn uppercase_clusters(
    paragraph: &str,
    range: Range<usize>,
    language: &str,
) -> Vec<(usize, String)> {
    let language = language
        .parse::<icu_locale_core::Locale>()
        .map_or(icu_locale_core::LanguageIdentifier::UNKNOWN, |locale| {
            locale.id
        });
    let mapper = CaseMapper::new();
    let mut context = mapper.context(paragraph);
    paragraph[range.clone()]
        .grapheme_indices(true)
        .map(|(start, cluster)| {
            let mut mapped = String::new();
            for (offset, _) in cluster.char_indices() {
                context
                    .map_char(
                        range.start + start + offset,
                        &language,
                        ContextCase::Upper,
                        &mut mapped,
                    )
                    .expect("valid source offsets and String writes cannot fail");
            }
            (start, mapped)
        })
        .collect()
}

/// Feature availability is decided per font, not per character (§6.6). Petite capitals fall
/// back to small capitals, unicase to small capitals for uppercase letters, and titling
/// capitals to nothing; small capitals are synthesized only where `synthesis` allows.
pub(crate) fn caps_plan(
    caps: FontVariantCaps,
    has: impl Fn([u8; 4]) -> bool,
    synthesis: bool,
) -> CapsPlan {
    let synthesized = |lower, upper| CapsPlan {
        features: &[],
        synthetic_lower: lower && synthesis,
        synthetic_upper: upper && synthesis,
    };
    let small = || has(SMALL_CAPITALS);
    let all_small = || has(SMALL_CAPITALS) && has(CAPITALS_TO_SMALL);
    match caps {
        FontVariantCaps::Normal => CapsPlan::NATIVE_NONE,
        FontVariantCaps::SmallCaps if small() => CapsPlan::native(SMALL_CAPS),
        FontVariantCaps::PetiteCaps if has(PETITE_CAPITALS) => CapsPlan::native(PETITE_CAPS),
        FontVariantCaps::PetiteCaps if small() => CapsPlan::native(SMALL_CAPS),
        FontVariantCaps::SmallCaps | FontVariantCaps::PetiteCaps => synthesized(true, false),
        FontVariantCaps::AllSmallCaps if all_small() => CapsPlan::native(ALL_SMALL_CAPS),
        FontVariantCaps::AllPetiteCaps if has(PETITE_CAPITALS) && has(CAPITALS_TO_PETITE) => {
            CapsPlan::native(ALL_PETITE_CAPS)
        }
        FontVariantCaps::AllPetiteCaps if all_small() => CapsPlan::native(ALL_SMALL_CAPS),
        FontVariantCaps::AllSmallCaps | FontVariantCaps::AllPetiteCaps => synthesized(true, true),
        FontVariantCaps::Unicase if has(UNICASE) => CapsPlan::native(UNICASE_CAPS),
        FontVariantCaps::Unicase => synthesized(false, true),
        FontVariantCaps::TitlingCaps if has(TITLING) => CapsPlan::native(TITLING_CAPS),
        FontVariantCaps::TitlingCaps => CapsPlan::NATIVE_NONE,
    }
}

/// The feature that renders `position` natively.
pub(crate) fn position_feature(position: FontVariantPosition) -> Option<FontFeature> {
    match position {
        FontVariantPosition::Normal => None,
        FontVariantPosition::Sub => Some(FontFeature::on(SUBSCRIPT)),
        FontVariantPosition::Super => Some(FontFeature::on(SUPERSCRIPT)),
    }
}

/// `font-variant-alternates` with indices that `lookup` defines for the face's family; names
/// it does not define select nothing (§6.8).
pub(crate) fn alternate_features<'a>(
    alternates: &FontVariantAlternates,
    lookup: impl Fn(FeatureValueType, &str) -> Option<&'a [u32]>,
) -> Vec<FontFeature> {
    let mut features = Vec::new();
    let single = |kind, name: &Option<Box<str>>| {
        name.as_deref()
            .and_then(|name| lookup(kind, name))
            .and_then(|values| values.first().copied())
    };
    if let Some(index) = single(FeatureValueType::Stylistic, &alternates.stylistic) {
        features.push(FontFeature {
            tag: STYLISTIC_ALTERNATES,
            value: index,
        });
    }
    if alternates.historical_forms {
        features.push(FontFeature::on(HISTORICAL_FORMS));
    }
    for name in &alternates.styleset {
        for set in lookup(FeatureValueType::Styleset, name).unwrap_or_default() {
            if let Some(tag) = numbered(STYLESET_PREFIX, *set) {
                features.push(FontFeature::on(tag));
            }
        }
    }
    for name in &alternates.character_variant {
        if let Some(&[variant, ref value @ ..]) = lookup(FeatureValueType::CharacterVariant, name)
            && let Some(tag) = numbered(CHARACTER_VARIANT_PREFIX, variant)
        {
            features.push(FontFeature {
                tag,
                value: value.first().copied().unwrap_or(FontFeature::ON),
            });
        }
    }
    if let Some(index) = single(FeatureValueType::Swash, &alternates.swash) {
        for tag in [SWASH, CONTEXTUAL_SWASH] {
            features.push(FontFeature { tag, value: index });
        }
    }
    for (kind, name, tag) in [
        (
            FeatureValueType::Ornaments,
            &alternates.ornaments,
            ORNAMENTS,
        ),
        (
            FeatureValueType::Annotation,
            &alternates.annotation,
            ANNOTATION,
        ),
    ] {
        if let Some(index) = single(kind, name) {
            features.push(FontFeature { tag, value: index });
        }
    }
    features
}

/// `ss01`–`ss99` or `cv01`–`cv99`.
fn numbered(prefix: [u8; 2], index: u32) -> Option<[u8; 4]> {
    if !NUMBERED_FEATURES.contains(&index) {
        return None;
    }
    let digit = |value: u32| b'0' + u8::try_from(value).expect("decimal digit");
    Some([
        prefix[0],
        prefix[1],
        digit(index / DECIMAL),
        digit(index % DECIMAL),
    ])
}

/// What the face contributes to a run's features.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FaceFeatures<'a> {
    /// The `font-feature-settings` descriptor of the face's `@font-face` rule.
    pub descriptor: &'a [FontFeature],
    /// `font-variant-alternates` as the face's family defines it.
    pub alternates: &'a [FontFeature],
    pub caps: &'static [FontFeature],
    /// The position feature, where the face renders the run's position natively.
    pub position: Option<FontFeature>,
}

/// The run's features in ascending precedence (§7.2): the `@font-face` descriptor, the
/// `font-variant` subproperties with `font-kerning`, ligature suppression for tracked text,
/// then `font-feature-settings`. Default features (§7.1) are the shaper's.
pub(crate) fn run_features(
    style: &Style,
    face: FaceFeatures<'_>,
    tracked: bool,
) -> Vec<FontFeature> {
    let features = &style.font_features;
    let mut list = face.descriptor.to_vec();
    match features.kerning {
        FontKerning::Auto => {}
        FontKerning::Normal => list.push(FontFeature::on(KERNING)),
        FontKerning::None => list.push(FontFeature::off(KERNING)),
    }
    list.extend_from_slice(&features.ligatures);
    list.extend(face.position);
    list.extend_from_slice(face.caps);
    list.extend_from_slice(&features.numeric);
    list.extend_from_slice(face.alternates);
    list.extend_from_slice(&features.east_asian);
    if tracked {
        list.extend_from_slice(&TRACKED_LIGATURES);
    }
    list.extend_from_slice(&features.settings);
    list
}
