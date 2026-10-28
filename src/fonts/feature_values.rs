//! `@font-feature-values` definitions (CSS Fonts 4 §6.9), looked up by the family of the
//! face that renders the text.
use crate::fonts::{FontBook, FontNamespace, matching};

/// A feature value block type of `@font-feature-values`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FeatureValueType {
    Stylistic,
    HistoricalForms,
    Styleset,
    CharacterVariant,
    Swash,
    Ornaments,
    Annotation,
}
impl FeatureValueType {
    /// Block at-keywords are ASCII case-insensitive.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        const NAMES: [(&str, FeatureValueType); 7] = [
            ("stylistic", FeatureValueType::Stylistic),
            ("historical-forms", FeatureValueType::HistoricalForms),
            ("styleset", FeatureValueType::Styleset),
            ("character-variant", FeatureValueType::CharacterVariant),
            ("swash", FeatureValueType::Swash),
            ("ornaments", FeatureValueType::Ornaments),
            ("annotation", FeatureValueType::Annotation),
        ];
        NAMES
            .iter()
            .find(|(candidate, _)| name.eq_ignore_ascii_case(candidate))
            .map(|(_, kind)| *kind)
    }
    /// Whether `values` is a valid definition of this type (§6.9.1, §6.9.2).
    pub(crate) fn accepts(self, values: &[u32]) -> bool {
        const CHARACTER_VARIANT_VALUES: std::ops::RangeInclusive<usize> = 1..=2;
        match self {
            Self::Stylistic | Self::Swash | Self::Ornaments | Self::Annotation => values.len() == 1,
            Self::CharacterVariant => CHARACTER_VARIANT_VALUES.contains(&values.len()),
            Self::Styleset | Self::HistoricalForms => !values.is_empty(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct FeatureValue {
    namespace: FontNamespace,
    family: String,
    kind: FeatureValueType,
    name: Box<str>,
    values: Box<[u32]>,
}

impl FontBook {
    /// Definitions are registered in document order; later ones replace earlier ones.
    pub(crate) fn define_feature_value(
        &mut self,
        namespace: FontNamespace,
        family: &str,
        kind: FeatureValueType,
        name: &str,
        values: Vec<u32>,
    ) {
        self.feature_values.push(FeatureValue {
            namespace,
            family: family.into(),
            kind,
            name: name.into(),
            values: values.into(),
        });
    }

    /// The indices `family` defines for `name`, in the innermost tree scope that defines it.
    /// Names are case-sensitive; family names match as font matching compares them.
    pub(crate) fn feature_value(
        &self,
        mut namespace: FontNamespace,
        family: &str,
        kind: FeatureValueType,
        name: &str,
    ) -> Option<&[u32]> {
        loop {
            if let Some(found) = self.feature_values.iter().rev().find(|value| {
                value.namespace == namespace
                    && value.kind == kind
                    && *value.name == *name
                    && matching::names_match(&value.family, family)
            }) {
                return Some(&found.values);
            }
            if namespace == FontNamespace::ROOT {
                return None;
            }
            namespace = self.namespaces[namespace.0].expect("child font namespace has a parent");
        }
    }
}
