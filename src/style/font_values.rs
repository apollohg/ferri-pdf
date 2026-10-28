use crate::style::{
    StyleError,
    component::Component,
    declaration::{self, Specified},
    grammar::{Keyword, Label, Production, Property},
    shorthands::longhand,
    wide_keywords::WideKeyword,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontVariantCaps {
    #[default]
    Normal,
    SmallCaps,
}

impl FontVariantCaps {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::SmallCaps) => Ok(Self::SmallCaps),
            _ => Err(crate::style::box_values::unsupported(value)),
        }
    }
}

pub(super) fn is_small_caps(value: &Component) -> bool {
    match value.unwrap() {
        Component::Keyword(Keyword::SmallCaps) => true,
        Component::Group(items) if items.len() == 1 => is_small_caps(&items[0]),
        Component::Slots(slots) => {
            let mut values = slots.iter().flatten();
            values.next().is_some_and(is_small_caps) && values.next().is_none()
        }
        _ => false,
    }
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
}
