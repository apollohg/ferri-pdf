//! CSS Fonts 4 §6–§9 feature, variation and palette values, and the `font` and
//! `font-variant` shorthands.
use crate::fonts::{FontFeature, FontPalette, FontVariation, SizeAdjust, SizeMetric};
use crate::style::{
    StyleError,
    box_values::unsupported,
    component::Component,
    declaration::{self, Specified},
    grammar::{Descriptor, Keyword, Label, Production, Property},
    shorthands::longhand,
    units::Relative,
    wide_keywords::WideKeyword,
};
use std::sync::Arc;

/// OpenType tags are four characters in U+20–7E (CSS Fonts 4 §6.12, §8.2).
const TAG_LENGTH: usize = 4;
const TAG_CHARACTERS: std::ops::RangeInclusive<char> = ' '..='~';
/// Language system tags shorter than four characters are padded with spaces (§6.13).
const TAG_PADDING: u8 = b' ';

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontVariantCaps {
    #[default]
    Normal,
    SmallCaps,
    AllSmallCaps,
    PetiteCaps,
    AllPetiteCaps,
    Unicase,
    TitlingCaps,
}

impl FontVariantCaps {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::SmallCaps) => Ok(Self::SmallCaps),
            Some(Keyword::AllSmallCaps) => Ok(Self::AllSmallCaps),
            Some(Keyword::PetiteCaps) => Ok(Self::PetiteCaps),
            Some(Keyword::AllPetiteCaps) => Ok(Self::AllPetiteCaps),
            Some(Keyword::Unicase) => Ok(Self::Unicase),
            Some(Keyword::TitlingCaps) => Ok(Self::TitlingCaps),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontKerning {
    /// Kerning is the UA's choice (§6.3); it applies the font's kerning as for `normal`.
    #[default]
    Auto,
    Normal,
    None,
}
impl FontKerning {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::None) => Ok(Self::None),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontVariantPosition {
    #[default]
    Normal,
    Sub,
    Super,
}
impl FontVariantPosition {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Sub) => Ok(Self::Sub),
            Some(Keyword::Super) => Ok(Self::Super),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontVariantEmoji {
    /// The UA's choice (§9.3): presentation follows the text and the fonts.
    #[default]
    Normal,
    Text,
    Emoji,
    Unicode,
}
impl FontVariantEmoji {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Text) => Ok(Self::Text),
            Some(Keyword::Emoji) => Ok(Self::Emoji),
            Some(Keyword::Unicode) => Ok(Self::Unicode),
            _ => Err(unsupported(value)),
        }
    }
}

/// The computed font feature and palette properties, which do not affect font matching.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub(crate) struct FontFeatures {
    pub kerning: FontKerning,
    pub ligatures: Arc<[FontFeature]>,
    pub position: FontVariantPosition,
    pub numeric: Arc<[FontFeature]>,
    pub alternates: Arc<FontVariantAlternates>,
    pub east_asian: Arc<[FontFeature]>,
    pub emoji: FontVariantEmoji,
    /// `font-feature-settings`, sorted by tag.
    pub settings: Arc<[FontFeature]>,
    pub language_override: Option<Box<str>>,
    pub palette: FontPalette,
}

/// `font-variant-alternates` with its font-specific names, which `@font-feature-values`
/// resolves per font family at glyph selection (§6.8).
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub(crate) struct FontVariantAlternates {
    pub historical_forms: bool,
    pub stylistic: Option<Box<str>>,
    pub styleset: Vec<Box<str>>,
    pub character_variant: Vec<Box<str>>,
    pub swash: Option<Box<str>>,
    pub ornaments: Option<Box<str>>,
    pub annotation: Option<Box<str>>,
}
impl FontVariantAlternates {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut alternates = Self::default();
        let mut parts = Vec::new();
        variant_parts(value, &mut parts);
        for part in parts {
            match part {
                Component::Keyword(Keyword::Normal) => {}
                Component::Keyword(Keyword::HistoricalForms) => alternates.historical_forms = true,
                Component::Function(function, _) => {
                    let names = feature_value_names(part);
                    let single = || names.first().cloned();
                    match function {
                        Keyword::Stylistic => alternates.stylistic = single(),
                        Keyword::Styleset => alternates.styleset = names,
                        Keyword::CharacterVariant => alternates.character_variant = names,
                        Keyword::Swash => alternates.swash = single(),
                        Keyword::Ornaments => alternates.ornaments = single(),
                        Keyword::Annotation => alternates.annotation = single(),
                        _ => return Err(unsupported(value)),
                    }
                }
                _ => return Err(unsupported(value)),
            }
        }
        Ok(alternates)
    }
}

fn feature_value_names(function: &Component) -> Vec<Box<str>> {
    function
        .arguments()
        .iter()
        .filter_map(|name| match name.unwrap() {
            Component::Ident(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Keywords and functional values in source order, through references, groups and slots.
fn variant_parts<'a>(value: &'a Component, output: &mut Vec<&'a Component>) {
    match value {
        Component::Keyword(_) | Component::Function(..) => output.push(value),
        Component::Reference(_, inner) => variant_parts(inner, output),
        Component::Group(items) => items.iter().for_each(|item| variant_parts(item, output)),
        Component::Slots(slots) => slots
            .iter()
            .flatten()
            .for_each(|slot| variant_parts(slot, output)),
        _ => {}
    }
}

const LIGATURES: Property = Property::FontVariantLigatures;
const NUMERIC: Property = Property::FontVariantNumeric;
const EAST_ASIAN: Property = Property::FontVariantEastAsian;
/// The property and OpenType features of each keyword of `font-variant-ligatures`,
/// `-numeric` and `-east-asian` (§6.4, §6.7, §6.10).
const KEYWORD_FEATURES: &[(Keyword, Property, &[FontFeature])] = &[
    (
        Keyword::CommonLigatures,
        LIGATURES,
        &[FontFeature::on(*b"liga"), FontFeature::on(*b"clig")],
    ),
    (
        Keyword::NoCommonLigatures,
        LIGATURES,
        &[FontFeature::off(*b"liga"), FontFeature::off(*b"clig")],
    ),
    (
        Keyword::DiscretionaryLigatures,
        LIGATURES,
        &[FontFeature::on(*b"dlig")],
    ),
    (
        Keyword::NoDiscretionaryLigatures,
        LIGATURES,
        &[FontFeature::off(*b"dlig")],
    ),
    (
        Keyword::HistoricalLigatures,
        LIGATURES,
        &[FontFeature::on(*b"hlig")],
    ),
    (
        Keyword::NoHistoricalLigatures,
        LIGATURES,
        &[FontFeature::off(*b"hlig")],
    ),
    (Keyword::Contextual, LIGATURES, &[FontFeature::on(*b"calt")]),
    (
        Keyword::NoContextual,
        LIGATURES,
        &[FontFeature::off(*b"calt")],
    ),
    // `none` disables every ligature and contextual feature the property covers.
    (
        Keyword::None,
        LIGATURES,
        &[
            FontFeature::off(*b"liga"),
            FontFeature::off(*b"clig"),
            FontFeature::off(*b"dlig"),
            FontFeature::off(*b"hlig"),
            FontFeature::off(*b"calt"),
        ],
    ),
    (Keyword::LiningNums, NUMERIC, &[FontFeature::on(*b"lnum")]),
    (Keyword::OldstyleNums, NUMERIC, &[FontFeature::on(*b"onum")]),
    (
        Keyword::ProportionalNums,
        NUMERIC,
        &[FontFeature::on(*b"pnum")],
    ),
    (Keyword::TabularNums, NUMERIC, &[FontFeature::on(*b"tnum")]),
    (
        Keyword::DiagonalFractions,
        NUMERIC,
        &[FontFeature::on(*b"frac")],
    ),
    (
        Keyword::StackedFractions,
        NUMERIC,
        &[FontFeature::on(*b"afrc")],
    ),
    (Keyword::Ordinal, NUMERIC, &[FontFeature::on(*b"ordn")]),
    (Keyword::SlashedZero, NUMERIC, &[FontFeature::on(*b"zero")]),
    (Keyword::Jis78, EAST_ASIAN, &[FontFeature::on(*b"jp78")]),
    (Keyword::Jis83, EAST_ASIAN, &[FontFeature::on(*b"jp83")]),
    (Keyword::Jis90, EAST_ASIAN, &[FontFeature::on(*b"jp90")]),
    (Keyword::Jis04, EAST_ASIAN, &[FontFeature::on(*b"jp04")]),
    (
        Keyword::Simplified,
        EAST_ASIAN,
        &[FontFeature::on(*b"smpl")],
    ),
    (
        Keyword::Traditional,
        EAST_ASIAN,
        &[FontFeature::on(*b"trad")],
    ),
    (Keyword::FullWidth, EAST_ASIAN, &[FontFeature::on(*b"fwid")]),
    (
        Keyword::ProportionalWidth,
        EAST_ASIAN,
        &[FontFeature::on(*b"pwid")],
    ),
    (Keyword::Ruby, EAST_ASIAN, &[FontFeature::on(*b"ruby")]),
];

fn keyword_features(keyword: Keyword) -> Option<(Property, &'static [FontFeature])> {
    KEYWORD_FEATURES
        .iter()
        .find(|(candidate, ..)| *candidate == keyword)
        .map(|(_, property, features)| (*property, *features))
}

/// `font-variant-ligatures`, `font-variant-numeric` or `font-variant-east-asian` as the
/// OpenType features it sets; `normal` sets none, leaving the defaults of §7.1.
pub(crate) fn variant_features(value: &Component) -> Result<Arc<[FontFeature]>, StyleError> {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    let mut features = Vec::new();
    for keyword in keywords
        .into_iter()
        .filter(|keyword| *keyword != Keyword::Normal)
    {
        let (_, set) = keyword_features(keyword).ok_or_else(|| unsupported(value))?;
        features.extend_from_slice(set);
    }
    Ok(features.into())
}

/// The tag of a valid `<opentype-tag>`.
fn opentype_tag(value: &Component) -> Option<[u8; 4]> {
    let Component::String(tag) = value.unwrap() else {
        return None;
    };
    (tag.chars().count() == TAG_LENGTH && tag.chars().all(|ch| TAG_CHARACTERS.contains(&ch)))
        .then(|| tag.as_bytes().try_into().expect("four ASCII characters"))
}

/// Parse-time validity beyond the grammar: OpenType tags must have exactly four
/// characters in U+20–7E, or the declaration is invalid (§6.12, §8.2).
pub(super) fn valid(property: Property, value: &Component) -> bool {
    if !matches!(
        property,
        Property::FontFeatureSettings | Property::FontVariationSettings
    ) || value.keyword() == Some(Keyword::Normal)
    {
        return true;
    }
    value
        .items()
        .iter()
        .all(|setting| setting.items().first().and_then(opentype_tag).is_some())
}

/// The tag validity of [`valid`] for the `@font-face` descriptors of the same names (§4.6).
pub(super) fn valid_descriptor(descriptor: Descriptor, value: &Component) -> bool {
    match descriptor {
        Descriptor::FontFaceFontFeatureSettings => valid(Property::FontFeatureSettings, value),
        Descriptor::FontFaceFontVariationSettings => valid(Property::FontVariationSettings, value),
        _ => true,
    }
}

/// `font-feature-settings`: a map from tags to values, sorted by tag (§6.12).
pub(crate) fn feature_settings(value: &Component) -> Result<Arc<[FontFeature]>, StyleError> {
    if value.keyword() == Some(Keyword::Normal) {
        return Ok(Arc::from([]));
    }
    let mut settings = std::collections::BTreeMap::new();
    for setting in value.items() {
        let parts = setting.items();
        let tag = parts
            .first()
            .and_then(opentype_tag)
            .ok_or_else(|| unsupported(value))?;
        let selector = parts.get(1).map(Component::items).unwrap_or_default();
        let feature_value = match selector.first().map(Component::unwrap) {
            None | Some(Component::Keyword(Keyword::On)) => FontFeature::ON,
            Some(Component::Keyword(Keyword::Off)) => FontFeature::OFF,
            Some(Component::Integer(index)) => {
                u32::try_from(*index).map_err(|_| unsupported(value))?
            }
            Some(_) => return Err(unsupported(value)),
        };
        settings.insert(tag, feature_value);
    }
    Ok(settings
        .into_iter()
        .map(|(tag, value)| FontFeature { tag, value })
        .collect())
}

/// `font-variation-settings`: a map from axis tags to values, sorted by tag (§8.2).
pub(crate) fn variation_settings(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Arc<[FontVariation]>, StyleError> {
    if value.keyword() == Some(Keyword::Normal) {
        return Ok(Arc::from([]));
    }
    let mut settings = std::collections::BTreeMap::new();
    for setting in value.items() {
        let parts = setting.items();
        let (Some(tag), Some(number)) = (parts.first().and_then(opentype_tag), parts.get(1)) else {
            return Err(unsupported(value));
        };
        settings.insert(tag, crate::style::values::number(number, relative)?);
    }
    Ok(settings
        .into_iter()
        .map(|(tag, value)| FontVariation { tag, value })
        .collect())
}

/// A `font-language-override` string as the OpenType language system tag it selects, padded
/// with spaces; `None` where no tag has that spelling (§6.13).
pub(crate) fn language_system_tag(name: &str) -> Option<[u8; 4]> {
    if name.is_empty()
        || name.len() > TAG_LENGTH
        || !name.chars().all(|ch| TAG_CHARACTERS.contains(&ch))
    {
        return None;
    }
    let mut tag = [TAG_PADDING; TAG_LENGTH];
    tag[..name.len()].copy_from_slice(name.as_bytes());
    Some(tag)
}

/// `font-language-override`: `None` for `normal`.
pub(crate) fn language_override(value: &Component) -> Result<Option<Box<str>>, StyleError> {
    match value.unwrap() {
        Component::Keyword(Keyword::Normal) => Ok(None),
        Component::String(name) => Ok(Some(name.clone())),
        _ => Err(unsupported(value)),
    }
}

/// `font-size-adjust` (CSS Fonts 5 §3.4); `from-font` computes to the metric of the element's
/// first available font.
pub(crate) fn size_adjust(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<SizeAdjust>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    let [metric, amount] = value.items() else {
        return Err(unsupported(value));
    };
    let metric = match metric.items().first().and_then(Component::keyword) {
        None | Some(Keyword::ExHeight) => SizeMetric::ExHeight,
        Some(Keyword::CapHeight) => SizeMetric::CapHeight,
        Some(Keyword::ChWidth) => SizeMetric::ChWidth,
        Some(Keyword::IcWidth) => SizeMetric::IcWidth,
        Some(Keyword::IcHeight) => SizeMetric::IcHeight,
        Some(_) => return Err(unsupported(value)),
    };
    let value = if amount.keyword() == Some(Keyword::FromFont) {
        relative.font.size_metric(metric)?
    } else {
        crate::style::values::number(amount, relative)?
    };
    Ok(Some(SizeAdjust { metric, value }))
}

/// `font-palette` (§9.1). Interpolating palettes with `palette-mix()` needs CSS Color 4 §13
/// colour interpolation, which colour values do not provide.
pub(crate) fn palette(value: &Component) -> Result<FontPalette, StyleError> {
    match value.unwrap() {
        Component::Keyword(Keyword::Normal) => Ok(FontPalette::Normal),
        Component::Keyword(Keyword::Light) => Ok(FontPalette::Light),
        Component::Keyword(Keyword::Dark) => Ok(FontPalette::Dark),
        Component::Ident(name) => Ok(FontPalette::Named(name.clone())),
        _ => Err(unsupported(value)),
    }
}

/// The `font-variant` longhand that a keyword or functional value of the shorthand sets.
fn variant_longhand(part: &Component) -> Option<Property> {
    Some(match part {
        Component::Function(..) | Component::Keyword(Keyword::HistoricalForms) => {
            Property::FontVariantAlternates
        }
        Component::Keyword(keyword) => match keyword {
            Keyword::Sub | Keyword::Super => Property::FontVariantPosition,
            Keyword::Text | Keyword::Emoji | Keyword::Unicode => Property::FontVariantEmoji,
            _ if FontVariantCaps::from_component(part).is_ok() => Property::FontVariantCaps,
            keyword => keyword_features(*keyword)?.0,
        },
        _ => return None,
    })
}

/// CSS Fonts 4 §6.11: `normal` resets every subproperty, `none` sets only the ligatures to
/// `none`, and other values set the subproperties they mention.
pub(super) fn expand_variant(value: &Component) -> Result<Vec<(Property, Specified)>, StyleError> {
    let mut values = declaration::wide(Property::FontVariant, WideKeyword::Initial);
    let mut assigned: Vec<(Property, Vec<Component>)> = Vec::new();
    let mut parts = Vec::new();
    variant_parts(value, &mut parts);
    for part in parts {
        if part.keyword() == Some(Keyword::Normal) {
            continue;
        }
        let property = variant_longhand(part).ok_or_else(|| unsupported(value))?;
        match assigned
            .iter_mut()
            .find(|(candidate, _)| *candidate == property)
        {
            Some((_, components)) => components.push(part.clone()),
            None => assigned.push((property, vec![part.clone()])),
        }
    }
    for (property, mut components) in assigned {
        let component = if components.len() == 1 {
            components.remove(0)
        } else {
            Component::Group(components)
        };
        values
            .iter_mut()
            .find(|(candidate, _)| *candidate == property)
            .expect("font-variant metadata contains its subproperties")
            .1 = longhand(property, &component);
    }
    Ok(values)
}

pub(super) fn expand_font(value: &Component) -> Result<Vec<(Property, Specified)>, StyleError> {
    let mut values = declaration::wide(Property::Font, WideKeyword::Initial);
    if matches!(
        value.keyword(),
        Some(
            Keyword::Caption
                | Keyword::Icon
                | Keyword::Menu
                | Keyword::MessageBox
                | Keyword::SmallCaption
                | Keyword::StatusBar
        )
    ) {
        return Ok(values);
    }
    for required in [Property::FontSize, Property::FontFamily] {
        if value.find(Label::Property(required)).is_none() {
            return Err(StyleError::Invalid(format!(
                "font shorthand has no {}",
                required.name()
            )));
        }
    }
    let mut assign = |property, component: &Component| {
        let specified = if component.keyword() == Some(Keyword::Normal) {
            Specified::Wide(WideKeyword::Initial)
        } else {
            longhand(property, component)
        };
        values
            .iter_mut()
            .find(|(candidate, _)| *candidate == property)
            .expect("font metadata contains its explicit longhands")
            .1 = specified;
    };
    for property in [
        Property::FontStyle,
        Property::FontWeight,
        Property::FontSize,
        Property::LineHeight,
        Property::FontFamily,
    ] {
        if let Some(component) = value.find(Label::Property(property)) {
            assign(property, component);
        }
    }
    for (property, production) in [
        (Property::FontVariantCaps, Production::FontVariantCss2),
        (Property::FontWidth, Production::FontWidthCss3),
    ] {
        if let Some(component) = value.find(Label::Production(production)) {
            assign(property, component);
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use crate::style::{
        declaration, declaration::Specified, grammar::Property, wide_keywords::WideKeyword,
    };
    use std::collections::BTreeSet;

    #[test]
    fn font_expansion_has_exactly_the_selected_explicit_and_reset_only_longhands() {
        let values = declaration::expand(
            Property::Font,
            "italic small-caps 753 condensed 18px/1.5 'Named Family', serif",
        )
        .unwrap()
        .unwrap();
        let names: BTreeSet<_> = values.iter().map(|(property, _)| property.name()).collect();
        assert_eq!(
            names,
            BTreeSet::from([
                "font-family",
                "font-size",
                "font-width",
                "font-style",
                "font-variant-caps",
                "font-weight",
                "line-height",
                "font-feature-settings",
                "font-kerning",
                "font-language-override",
                "font-optical-sizing",
                "font-size-adjust",
                "font-variant-alternates",
                "font-variant-east-asian",
                "font-variant-emoji",
                "font-variant-ligatures",
                "font-variant-numeric",
                "font-variant-position",
                "font-variation-settings",
            ])
        );
        assert_eq!(
            values.len(),
            names.len(),
            "each longhand must be emitted once"
        );
        for (property, expected) in [
            (Property::FontStyle, "italic"),
            (Property::FontVariantCaps, "small-caps"),
            (Property::FontWeight, "753"),
            (Property::FontWidth, "condensed"),
            (Property::FontSize, "18px"),
            (Property::LineHeight, "1.5"),
        ] {
            let specified = &values.iter().find(|(name, _)| *name == property).unwrap().1;
            let Specified::Value(value) = specified else {
                panic!(
                    "{} must expand to its explicit value, got {specified:?}",
                    property.name()
                )
            };
            assert_eq!(value.to_string(), expected, "{}", property.name());
        }
        for (property, specified) in &values {
            if ![
                Property::FontStyle,
                Property::FontVariantCaps,
                Property::FontWeight,
                Property::FontWidth,
                Property::FontSize,
                Property::LineHeight,
                Property::FontFamily,
            ]
            .contains(property)
            {
                assert!(
                    matches!(specified, Specified::Wide(WideKeyword::Initial)),
                    "{} must be reset to its own initial value: {specified:?}",
                    property.name()
                );
            }
        }
        let Specified::Value(family) = &values
            .iter()
            .find(|(name, _)| *name == Property::FontFamily)
            .unwrap()
            .1
        else {
            panic!("font-family must expand")
        };
        assert_eq!(
            crate::style::values::families(family),
            vec![
                crate::fonts::FontFamily::Named("Named Family".into()),
                crate::fonts::FontFamily::Generic(crate::fonts::GenericFamily::Serif)
            ]
        );
    }

    #[test]
    fn font_width_alias_and_wide_keywords_use_the_same_canonical_longhands() {
        for property in [Property::FontWidth, Property::FontStretch] {
            assert!(
                declaration::expand(property, "-20%").unwrap().is_none(),
                "{} must reject the same out-of-range percentage",
                property.name()
            );
        }
        let width = declaration::expand(Property::FontStretch, "condensed")
            .unwrap()
            .unwrap();
        assert_eq!(width.len(), 1);
        assert_eq!(width[0].0, Property::FontWidth);
        assert!(
            matches!(width[0].1, Specified::Value(_)),
            "the legacy name must be a real alias"
        );
        for source in ["initial", "inherit", "unset", "revert", "revert-layer"] {
            let values = declaration::expand(Property::Font, source)
                .unwrap()
                .unwrap();
            assert_eq!(
                values.len(),
                19,
                "{source} must cover explicit and reset-only longhands"
            );
            assert!(values.iter().all(|(_, value)| matches!(value, Specified::Wide(keyword) if *keyword == WideKeyword::parse(source).unwrap())));
        }
    }

    /// Each longhand's value as `font-variant` expands it; `initial` where it is reset.
    fn variant_longhands(source: &str) -> Vec<(&'static str, String)> {
        let values = declaration::expand(Property::FontVariant, source)
            .unwrap()
            .unwrap_or_else(|| panic!("font-variant: {source} must be valid"));
        assert_eq!(values.len(), 7, "{source} must set every subproperty");
        values
            .into_iter()
            .map(|(property, value)| {
                let value = match value {
                    Specified::Value(value) => value.to_string(),
                    Specified::Wide(WideKeyword::Initial) => "initial".into(),
                    other => panic!("{source}: unexpected {} value {other:?}", property.name()),
                };
                (property.name(), value)
            })
            .collect()
    }

    #[test]
    fn font_variant_sets_the_subproperties_its_values_belong_to() {
        let initial = |name| (name, "initial".to_owned());
        assert_eq!(
            variant_longhands("normal"),
            [
                "font-variant-alternates",
                "font-variant-caps",
                "font-variant-east-asian",
                "font-variant-emoji",
                "font-variant-ligatures",
                "font-variant-numeric",
                "font-variant-position",
            ]
            .map(initial)
        );
        let none = variant_longhands("none");
        assert_eq!(
            none.iter()
                .filter(|(_, value)| value != "initial")
                .collect::<Vec<_>>(),
            [&("font-variant-ligatures", "none".to_owned())],
            "none sets only the ligatures (§6.11)"
        );
        let all = variant_longhands(
            "no-common-ligatures styleset(a, b) all-small-caps tabular-nums discretionary-ligatures ruby super emoji jis04 historical-forms slashed-zero",
        );
        let value = |name| {
            all.iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, value)| value.as_str())
                .unwrap()
        };
        assert_eq!(
            value("font-variant-ligatures"),
            "no-common-ligatures discretionary-ligatures"
        );
        assert_eq!(
            value("font-variant-alternates"),
            "historical-forms styleset(a b)"
        );
        assert_eq!(value("font-variant-caps"), "all-small-caps");
        assert_eq!(value("font-variant-numeric"), "tabular-nums slashed-zero");
        assert_eq!(value("font-variant-east-asian"), "jis04 ruby");
        assert_eq!(value("font-variant-position"), "super");
        assert_eq!(value("font-variant-emoji"), "emoji");
        for invalid in [
            "sub super",
            "small-caps unicase",
            "normal small-caps",
            "none ruby",
        ] {
            assert!(
                declaration::expand(Property::FontVariant, invalid)
                    .unwrap()
                    .is_none(),
                "font-variant: {invalid} combines exclusive values and must be invalid"
            );
        }
    }

    #[test]
    fn opentype_tags_and_selectors_are_validated_at_parse_time() {
        for (property, source) in [
            (Property::FontFeatureSettings, "'silly' off"),
            (Property::FontFeatureSettings, "'abc'"),
            (Property::FontFeatureSettings, "'t\u{e9}st'"),
            (Property::FontFeatureSettings, "sinf"),
            (Property::FontFeatureSettings, "'sinf' 'twid'"),
            (Property::FontFeatureSettings, "'sinf' -1"),
            (Property::FontVariationSettings, "'wght'"),
            (Property::FontVariationSettings, "'weight' 700"),
            (Property::FontVariationSettings, "'wght' 700 'wdth' 50"),
        ] {
            assert!(
                declaration::expand(property, source).unwrap().is_none(),
                "{}: {source} must be invalid",
                property.name()
            );
        }
        let settings = |source| {
            let values = declaration::expand(Property::FontFeatureSettings, source)
                .unwrap()
                .unwrap_or_else(|| panic!("font-feature-settings: {source} must be valid"));
            let Specified::Value(value) = &values[0].1 else {
                panic!("{source} must be a value")
            };
            super::feature_settings(value)
                .unwrap()
                .iter()
                .map(|feature| {
                    (
                        String::from_utf8_lossy(&feature.tag).into_owned(),
                        feature.value,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            settings("'zero' off, 'sinf', 'aalt' 3, 'zero' on, ' ~!x' 0"),
            [(" ~!x", 0), ("aalt", 3), ("sinf", 1), ("zero", 1)]
                .map(|(tag, value)| (tag.to_owned(), value)),
            "the computed map keeps the last value of each tag, sorted by code unit (§6.12)"
        );
        assert_eq!(settings("normal"), Vec::<(String, u32)>::new());
    }

    #[test]
    fn language_system_tags_pad_short_names_and_reject_impossible_ones() {
        assert_eq!(super::language_system_tag("SRB"), Some(*b"SRB "));
        assert_eq!(super::language_system_tag("TRK "), Some(*b"TRK "));
        assert_eq!(super::language_system_tag("IPPH"), Some(*b"IPPH"));
        for impossible in ["", "LONGER", "T\u{dc}R"] {
            assert_eq!(
                super::language_system_tag(impossible),
                None,
                "{impossible:?} cannot name a four-byte tag"
            );
        }
    }
}
