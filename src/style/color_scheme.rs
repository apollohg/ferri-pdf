//! CSS Color Adjustment 1 §2 `color-scheme` and the used colour scheme it negotiates.
use crate::dom::{DocumentTree, HTML_NAMESPACE};
use crate::style::{
    ColorScheme,
    component::Component,
    grammar::{Keyword, Property},
};

const META_ELEMENT: &str = "meta";
const NAME_ATTRIBUTE: &str = "name";
const CONTENT_ATTRIBUTE: &str = "content";
const COLOR_SCHEME_NAME: &str = "color-scheme";

/// Computed `color-scheme`. Custom idents have no meaning and `only` forbids user-agent
/// overrides, which print preferences never request, so neither affects the used scheme.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub(crate) enum ColorSchemes {
    #[default]
    Normal,
    /// The recognised schemes in specified order.
    Listed(Vec<ColorScheme>),
}

impl ColorSchemes {
    pub(crate) fn from_component(value: &Component) -> Self {
        if value.keyword() == Some(Keyword::Normal) {
            return Self::Normal;
        }
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        Self::Listed(
            keywords
                .into_iter()
                .filter_map(|keyword| match keyword {
                    Keyword::Light => Some(ColorScheme::Light),
                    Keyword::Dark => Some(ColorScheme::Dark),
                    _ => None,
                })
                .collect(),
        )
    }

    /// `normal` takes the page's supported schemes (HTML's `color-scheme` meta element); the
    /// preferred scheme wins when listed, then the first listed, then the light default.
    pub(crate) fn used(&self, preferred: ColorScheme, page: &Self) -> ColorScheme {
        let listed = match (self, page) {
            (Self::Listed(listed), _) | (Self::Normal, Self::Listed(listed)) => listed.as_slice(),
            (Self::Normal, Self::Normal) => &[],
        };
        if listed.contains(&preferred) {
            preferred
        } else {
            listed.first().copied().unwrap_or_default()
        }
    }
}

/// HTML §4.2.5.1: the page's supported colour schemes come from the first `color-scheme`
/// meta element whose content is a valid `color-scheme` value.
pub(crate) fn page_color_schemes(tree: &DocumentTree) -> ColorSchemes {
    tree.elements()
        .filter(|element| {
            element.is_in_document_tree()
                && element.namespace() == HTML_NAMESPACE
                && element.name() == META_ELEMENT
                && element
                    .attr(NAME_ATTRIBUTE)
                    .is_some_and(|name| name.eq_ignore_ascii_case(COLOR_SCHEME_NAME))
        })
        .filter_map(|element| element.attr(CONTENT_ATTRIBUTE))
        .find_map(|content| {
            crate::style::declaration::value(Property::ColorScheme, content, &mut 0)
                .ok()
                .flatten()
                .map(|value| ColorSchemes::from_component(&value))
        })
        .unwrap_or_default()
}
