use crate::style::{
    StyleError,
    box_values::unsupported,
    component::Component,
    grammar::{Keyword, Label, Production},
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ListStyleType {
    None,
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerRoman,
    UpperRoman,
    LowerGreek,
    LowerAlpha,
    UpperAlpha,
}
impl ListStyleType {
    /// A predefined counter style name (CSS Counter Styles 3 §6), ASCII case-insensitive.
    pub(super) fn named(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "none" => Self::None,
            "disc" => Self::Disc,
            "circle" => Self::Circle,
            "square" => Self::Square,
            "decimal" => Self::Decimal,
            "decimal-leading-zero" => Self::DecimalLeadingZero,
            "lower-roman" => Self::LowerRoman,
            "upper-roman" => Self::UpperRoman,
            "lower-greek" => Self::LowerGreek,
            "lower-alpha" | "lower-latin" => Self::LowerAlpha,
            "upper-alpha" | "upper-latin" => Self::UpperAlpha,
            _ => return None,
        })
    }
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::None);
        }
        match value
            .find(Label::Production(Production::CounterStyleName))
            .map(Component::unwrap)
        {
            Some(Component::Ident(name)) => Self::named(name).ok_or_else(|| unsupported(value)),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}
impl ListStylePosition {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Outside) => Ok(Self::Outside),
            Some(Keyword::Inside) => Ok(Self::Inside),
            _ => Err(unsupported(value)),
        }
    }
}
