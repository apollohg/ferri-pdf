use crate::style::{
    StyleError,
    box_values::unsupported,
    component::Component,
    counter_style::{CounterStyle, Scope},
    grammar::Keyword,
};
use serde::Serialize;
use std::sync::Arc;

const INITIAL_LIST_STYLE_TYPE: &str = "disc";

/// The computed `list-style-type` (CSS Lists 3 §3.4).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum ListStyleType {
    None,
    String(Arc<str>),
    Counter(CounterStyle),
}
impl Default for ListStyleType {
    fn default() -> Self {
        Self::Counter(CounterStyle::predefined(INITIAL_LIST_STYLE_TYPE))
    }
}
impl ListStyleType {
    pub(super) fn from_component(value: &Component, scope: &Scope<'_>) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::None);
        }
        if let Component::String(text) = value.unwrap() {
            return Ok(Self::String(Arc::from(&**text)));
        }
        Ok(match scope.resolve(value)? {
            CounterStyle::None => Self::None,
            style => Self::Counter(style),
        })
    }
    pub(crate) fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
    /// The marker string of a list item whose `list-item` counter is `ordinal`, or `None`
    /// when there is none.
    pub(crate) fn marker(&self, ordinal: i128, rtl: bool) -> Result<Option<String>, StyleError> {
        match self {
            Self::None => Ok(None),
            Self::String(text) => Ok(Some(text.to_string())),
            Self::Counter(style) => style.marker(ordinal, rtl).map(Some),
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
